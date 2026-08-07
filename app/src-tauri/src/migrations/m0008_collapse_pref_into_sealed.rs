// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Migration `0008_collapse_pref_into_sealed` (R074).
//!
//! The **inverse of `m0005`**: collapse the plaintext `pref.json` (display) and
//! the sealed behavior slot (`app.json`, AAD `"app_behavior"`) into a SINGLE
//! sealed merged `app.json` (AAD `"app_config"`), then delete `pref.json`. R064
//! made the at-rest master key auth-free, so the premise for the split (display
//! prefs had to be plaintext-readable before the key was available) is gone —
//! the config tier now holds **zero plaintext**.
//!
//! Algorithm (write order is the commit point):
//!
//! ```text
//! unseal app.json ─► AppConfig@8? ─yes─► (already merged) ─► delete pref.json, Done
//!                       │no
//!                       └─► BehaviorConfig@7 ─merge w/ pref─►
//!                            write sealed merged app.json@8 ─delete pref.json─► Done
//! ```
//!
//! 1. Read the display half from plaintext `pref.json` (`get_pref`).
//! 2. **Single-unseal typed dispatch** (decision D3): unseal `app.json` once via
//!    the dual-AAD `load_app_config` (reads the new `"app_config"` tag OR falls
//!    back to the legacy `"app_behavior"` tag m0005/m0006 wrote) → try
//!    `AppConfig` at schema ≥ 8 (already merged: a prior run crashed before
//!    deleting `pref.json`) ⇒ treat it as authoritative, fall through to delete;
//!    else parse as `BehaviorConfig` (the schema-7 shape) ⇒ the behavior half.
//! 3. Merge into `AppConfig` (schema 8) and write the sealed merged `app.json`,
//!    THEN delete `pref.json` — the delete is the commit. A crash between is
//!    recoverable: re-run sees `pref.json`@7, step 2 detects the already-merged
//!    `app.json`@8, deletes `pref.json`, no re-seal.
//!
//! **No app-lock defer** (decision D): the auth-free master key is loaded at
//! `.setup()` always (including App Lock), so `has_master_key()` is true here.
//! The vault key (the identity gate) stays biometric-gated — `m0007` still
//! defers on it, but `m0008` runs only after `m0007` completes (the engine is
//! ordered), so by here the chain is unblocked.
//!
//! Idempotent (gated on `schema_version`) and safe to call on every startup and
//! `app_unlock`.

use std::io;

use rustpass::Error;
use tokio::fs;

use crate::AppState;
use crate::app_config::{AppConfig, BehaviorConfig};
use crate::migrations::MigrationOutcome;

/// Collapse `pref.json` + the sealed behavior slot into a single sealed merged
/// `app.json`, bumping `schema_version` to 8. See the module docs for the
/// algorithm, the half-migrated recovery, and the (absent) app-lock defer.
///
/// Outcomes:
/// - missing `pref.json` ⇒ unreachable in practice (the engine gates on a
///   pref.json-sourced peek at schema < 8); defensively `Done`.
/// - `app.json` already a merged `AppConfig`@8 (half-migrated recovery) ⇒ delete
///   `pref.json`, `Done` (no re-seal).
/// - `app.json` absent (`NO_IDENTITY`) ⇒ merge pref + default behavior, write,
///   delete pref.json, `Done`.
/// - `app.json` unparseable as either shape (AEAD-valid garbage) ⇒ `Err`
///   (pref.json preserved; the app stays in the old-world layout rather than
///   forcing re-setup — the merged app.json was never written, so a `Done`
///   would peek `Corrupt` next launch and wipe the display prefs).
/// - unseal error other than `NO_IDENTITY` (tamper / lost key) ⇒ `Err` (engine
///   retries; persistent = re-setup). Never silent-default behavior prefs.
/// - the sealed write or the pref.json delete failing ⇒ `Err` (engine retries;
///   schema stays below target via the surviving `pref.json`).
pub(crate) async fn apply(state: &AppState, version: u32) -> Result<MigrationOutcome, Error> {
    // 1. Display half from plaintext pref.json (the in-memory pref cache,
    //    populated by new() from pref.json at schema < 8).
    let pref = state.app_config.get_pref();

    // 2. Single-unseal the app.json slot (dual-AAD: new app_config tag OR legacy
    //    app_behavior tag).
    match state.store.load_app_config().await {
        Ok(bytes) => {
            // D3 typed dispatch: a file the prior run already merged parses as an
            // AppConfig at schema >= 8 — it is authoritative, so skip the re-seal
            // and just drop the stale pref.json.
            if let Ok(existing) = serde_json::from_slice::<AppConfig>(&bytes)
                && existing.schema_version >= version
            {
                return delete_pref_json(state).await;
            }
            // Otherwise parse the schema-7 behavior shape.
            let behavior: BehaviorConfig = match serde_json::from_slice(&bytes) {
                Ok(b) => b,
                Err(e) => {
                    // AEAD-valid but unparseable as either shape: the sealed
                    // content is unusable (a sealing-code bug or an
                    // incompatible downgrade). Propagate Err so pref.json
                    // survives for recovery and the app keeps running in the
                    // old-world layout — DON'T delete pref.json + Done, which
                    // would force re-setup with the display prefs gone (the
                    // merged app.json was never written, so the next peek would
                    // read Corrupt).
                    log::warn!(
                        "0008_collapse: app.json unparseable as AppConfig or BehaviorConfig ({e}); surfacing Err, pref.json left intact"
                    );
                    return Err(e.into());
                }
            };
            write_merged_and_delete_pref(state, version, &pref, &behavior).await?;
        }
        Err(e) if e.code == "NO_IDENTITY" => {
            // No behavior slot — merge pref + default behavior.
            write_merged_and_delete_pref(state, version, &pref, &BehaviorConfig::default()).await?;
        }
        Err(e) if e.code == "SEAL_KEY_UNAVAILABLE" => {
            // Unreachable under decision D (auth-free key loaded at .setup()), but
            // surface it so the engine retries rather than silently skipping.
            return Err(e);
        }
        Err(e) => {
            // Tamper / corrupt: propagate as Err (D5) — persistent failure routes
            // to re-setup. Never silent-default security-relevant behavior prefs.
            return Err(e);
        }
    }

    Ok(MigrationOutcome::Done)
}

/// Build the merged `AppConfig` at `version`, seal it into `app.json`, then
/// delete `pref.json`. The sealed write lands first (atomic); the delete is the
/// commit point. A crash between leaves `pref.json`@<8 + merged `app.json`@8,
/// which a re-run detects via the typed dispatch (no re-seal, just the delete).
async fn write_merged_and_delete_pref(
    state: &AppState,
    version: u32,
    pref: &crate::app_config::PrefConfig,
    behavior: &BehaviorConfig,
) -> Result<(), Error> {
    let mut merged = AppConfig::from_halves(pref, behavior);
    merged.schema_version = version;
    let json = serde_json::to_string_pretty(&merged)?;
    // Write the sealed merged app.json BEFORE deleting pref.json (commit point).
    state.store.save_app_config(json.as_bytes()).await?;
    delete_pref_json(state).await?;
    Ok(())
}

/// Delete `pref.json`. Propagates a failure as `Err` so the engine retries — a
/// surviving `pref.json`@<8 would keep the schema below target and re-trigger
/// this migration until the delete lands. `Done` on success or already-absent.
async fn delete_pref_json(state: &AppState) -> Result<MigrationOutcome, Error> {
    match fs::remove_file(state.app_config.pref_json_path()).await {
        Ok(()) => Ok(MigrationOutcome::Done),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(MigrationOutcome::Done),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    // The migration's behavior is exercised end-to-end via the migration
    // registry tests (schema 7 → 8, pref.json deleted, values preserved,
    // idempotent re-run, half-migrated recovery) in `tests::migrations`.
}
