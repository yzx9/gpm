// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Migration `0005_split_app_json`.
//!
//! **Historical note (R074):** `m0008_collapse_pref_into_sealed` is the inverse
//! of this migration — it merges `pref.json` + the sealed behavior slot back into
//! a single sealed `app.json` and deletes `pref.json`. This migration stays in
//! the permanent registry so schema-<8 upgraders still pass through the split on
//! their way to the collapsed schema-8 shape.
//!
//! Splits the single plaintext `app.json` (the schema-4 single-file shape that
//! `m0002`/`m0003`/`m0004_verbose_from_debug` last wrote — read as
//! [`AppConfigV4`]) into the post-split pair:
//! - **`pref.json`** (plaintext) — display prefs (`locale`, `theme_mode`,
//!   `verbose_until`, `schema_version`). No deprecated `secure_screen`/`log_level`
//!   — those were consumed by `m0003`/`m0004` before reaching V4.
//! - **`app.json`** (sealed via `Store::save_app_behavior`) — behavior prefs
//!   (`lock_mode`, the view/clipboard clear timers, `autosync`,
//!   `biometric_app_lock`, `secure_screen_mode`).
//!
//! Same file name (`app.json`) is reused for the sealed behavior slot — the
//! sealed slot is distinguished by AAD (`"app_behavior"`), not by path. On
//! desktop (master key `None`) the seal is passthrough-plaintext, so the
//! post-split `app.json` is plaintext JSON of the [`BehaviorConfig`] shape; on
//! Android it is an AEAD envelope (unreadable until unlock).
//!
//! Idempotent (the engine gates on `schema_version`, which lives in
//! `pref.json` post-split) and safe to call on every startup and `app_unlock`.
//!
//! # App-lock resume
//!
//! The display half (`pref.json`) is always writeable: it is plaintext, so
//! there is no Seal/key dependency. The behavior half (`app.json`) goes through
//! the Seal, which gates on the master key: passthrough on desktop, sealed on
//! Android. Under the app-launch biometric lock the master key is wiped at
//! cold start (injected only after the biometric prompt), so the sealed write
//! would fail `SealKeyUnavailable` — the migration must defer (`Pending`) so
//! the next `app_unlock` retries from the top. The corrected guard is
//! `app_lock_enabled && !has_master_key()`: `app_locked` is unreliable here
//! because `run_app_migrations` runs inside `app_unlock` AFTER the key is
//! injected but BEFORE `app_locked` is cleared (so `app_locked` is still
//! `true` even though the key is now in memory).

use std::io;
use std::sync::atomic::Ordering;

use rustpass::Error;
use tokio::fs;

use crate::AppState;
use crate::app_config::{BehaviorConfig, GateIdle, PrefConfig};
use crate::migrations::MigrationOutcome;
use crate::migrations::m0004_verbose_from_debug::AppConfigV4;

/// Split the schema-4 plaintext single-file `app.json` (read as [`AppConfigV4`])
/// into `pref.json` + sealed `app.json`, bumping `schema_version` to 5. See the
/// module docs for the app-lock resume semantics.
///
/// Outcomes:
/// - missing `app.json` (fresh install / post-reset) → write `pref.json` with
///   `schema_version` bumped to 5, return `Done` (no behavior to seal).
/// - `app.json` already an envelope (half-migrated recovery: a prior run sealed
///   the behavior half but crashed before bumping the schema) → bump `pref.json`
///   schema to 5, return `Done`.
/// - unparseable as V4 (unknown shape; main-shipped schema-4 files carrying
///   deprecated keys still parse via no-`deny_unknown_fields`) → warn + mark
///   done (mirrors `new()` resilience — the user can recover via a hand edit,
///   and we must not brick the startup loop).
/// - `app_lock_enabled && !has_master_key()` → `Pending` (the master key is
///   wiped at cold start under the app-launch gate; the next `app_unlock`
///   retries from the top).
/// - `SealKeyUnavailable` from the sealed behavior write → `Pending`
///   (defensive; the guard should have caught it).
/// - otherwise → split, bump schema to 5, re-seed the `Store`'s `autosync`
///   cache, `Done`. A pref-write failure between the behavior write and the
///   schema bump is propagated as `Err` so the engine retries (the schema
///   stays at the preserved value; the sealed behavior write is idempotent
///   because it overwrites).
pub(crate) async fn apply(state: &AppState, version: u32) -> Result<MigrationOutcome, Error> {
    // 1. The engine gates on `peek_schema_version` (raw disk read), so an
    //    idempotent re-entry guard here would be redundant. NOTE: do NOT gate
    //    on `state.app_config.get_pref().schema_version` — the pref cache
    //    starts at `PrefConfig::default()` (target schema) when
    //    `AppConfigStore::new` could not legacy-lift a corrupt app.json, which
    //    would short-circuit m0005 and strand app.json below target.

    // 2. Read raw app.json bytes from disk.
    let app_json_path = state.app_config.app_json_path();
    let bytes = match fs::read(app_json_path).await {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // 3. Missing — fresh install / post-reset. Nothing to split; bump
            //    pref schema and mark done.
            return bump_pref_schema_to(state, version)
                .await
                .map(|()| MigrationOutcome::Done);
        }
        Err(e) => {
            // Unreadable: warn + mark done (mirrors `new()` resilience — the
            // user can recover via a hand edit, and we must not brick the
            // startup loop).
            log::warn!("0005_split_app_json: app.json unreadable ({e}); marking done");
            return bump_pref_schema_to(state, version)
                .await
                .map(|()| MigrationOutcome::Done);
        }
    };

    // 4. Already-sealed envelope: half-migrated recovery. Bump pref schema and
    //    mark done — the sealed behavior half is already in place.
    if rustpass::seal::is_envelope(&bytes) {
        return bump_pref_schema_to(state, version)
            .await
            .map(|()| MigrationOutcome::Done);
    }

    // 5. Plaintext V4: parse via the read_app_json_as path (which re-reads via
    //    tokio::fs — bytes were only needed for is_envelope above; re-reading is
    //    cheap). V4 has no `deny_unknown_fields`, so a main-shipped schema-4
    //    file carrying the deprecated `secure_screen` and/or `log_level` keys
    //    parses cleanly (the unknown keys are ignored).
    let v4: AppConfigV4 = match state.app_config.read_app_json_as().await {
        Ok(c) => c,
        Err(e) => {
            // Unparseable as V4: warn + mark done. The file is in an unknown
            // shape; rather than brick startup, treat it as migrated.
            log::warn!("0005_split_app_json: app.json unparseable as V4 ({e}); marking done");
            return bump_pref_schema_to(state, version)
                .await
                .map(|()| MigrationOutcome::Done);
        }
    };

    // Write the display half to pref.json FIRST, but ONLY on the first run
    // (pref.json absent). Once pref.json exists the display half is already
    // split and pref.json is authoritative for it — re-deriving display prefs
    // from app.json would clobber the user's locale/theme on desktop, where a
    // half-migrated app.json (the sealed write landed but the schema-bump
    // crashed) is a plaintext `BehaviorConfig` that parses as a degenerate V4
    // with defaulted display fields + `schema_version: 4`. PRESERVE
    // schema_version (do NOT bump yet) — the schema advances only after the
    // sealed write succeeds, so a Pending resume re-enters cleanly.
    if !state.app_config.pref_json_exists().await {
        state
            .app_config
            .save_pref(&PrefConfig {
                locale: v4.locale.clone(),
                theme_mode: v4.theme_mode.clone(),
                verbose_until: v4.verbose_until,
                schema_version: v4.schema_version,
                ..PrefConfig::default()
            })
            .await?;
    }

    // 6. App-lock guard: master key not yet injected. Defer — the next
    //    app_unlock retries from the top (after biometric injects the key).
    if state.app_lock_enabled.load(Ordering::SeqCst) && !state.store.has_master_key() {
        return Ok(MigrationOutcome::Pending);
    }

    // 7. Build behavior from V4 and seal it into app.json.
    let behavior = BehaviorConfig {
        lock_mode: v4.lock_mode,
        view_clear_secs: v4.view_clear_secs,
        clipboard_clear_secs: v4.clipboard_clear_secs,
        autosync: v4.autosync,
        biometric_app_lock: v4.biometric_app_lock,
        secure_screen_mode: v4.secure_screen_mode,
        // gate_idle is new (not in V4); default it, and m0006 pins existing
        // users to Off on the next step.
        gate_idle: GateIdle::default(),
    };
    if let Err(e) = state.app_config.save_behavior(&behavior).await {
        if e.code == "SEAL_KEY_UNAVAILABLE" {
            // Defensive: the guard should have caught this, but a race with
            // app_lock mid-write surfaces here. Defer to the next app_unlock.
            return Ok(MigrationOutcome::Pending);
        }
        return Err(e);
    }

    // 8. Bump pref schema to target. Propagating a failure here surfaces as
    //    Err so the engine retries (schema stays at the preserved value; the
    //    behavior cache was already populated by `save_behavior` above).
    bump_pref_schema_to(state, version).await?;

    // Mirror m0002: re-seed the Store's injected autosync cache so
    // autosync_write reads the persisted value, not the cold-start default.
    state.store.set_autosync(behavior.autosync);

    Ok(MigrationOutcome::Done)
}

/// Read the current pref cache, bump its `schema_version` to `version`, and
/// persist atomically. Propagates a save failure so the engine retries — never
/// marks Done without persisting (the engine's `debug_assert_eq!` would fire).
async fn bump_pref_schema_to(state: &AppState, version: u32) -> Result<(), Error> {
    let mut pref = state.app_config.get_pref();
    pref.schema_version = version;
    state.app_config.save_pref(&pref).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `BehaviorConfig` extraction from a V4 must pull all six behavior fields
    /// off the parsed V4. The cross-module
    /// `behavior_config_from_legacy_preserves_behavior_fields` test in
    /// `app_config::tests` already pins the `LegacyAppConfig` equivalent; this
    /// test exists as a local mirror so a refactor that breaks the V4 → behavior
    /// extraction fails inside the migration's own tests (closer to the bug).
    #[test]
    fn v4_to_behavior_pulls_all_six_behavior_fields() {
        use crate::app_config::SecureScreenMode;
        let v4 = AppConfigV4 {
            secure_screen_mode: Some(SecureScreenMode::Always),
            lock_mode: rustpass::LockMode::Idle(120),
            view_clear_secs: Some(0),
            clipboard_clear_secs: Some(180),
            autosync: false,
            biometric_app_lock: true,
            ..Default::default()
        };
        let b = BehaviorConfig {
            lock_mode: v4.lock_mode,
            view_clear_secs: v4.view_clear_secs,
            clipboard_clear_secs: v4.clipboard_clear_secs,
            autosync: v4.autosync,
            biometric_app_lock: v4.biometric_app_lock,
            secure_screen_mode: v4.secure_screen_mode,
            gate_idle: GateIdle::default(),
        };
        assert_eq!(b.secure_screen_mode, Some(SecureScreenMode::Always));
        assert_eq!(b.lock_mode, rustpass::LockMode::Idle(120));
        assert_eq!(b.view_clear_secs, Some(0));
        assert_eq!(b.clipboard_clear_secs, Some(180));
        assert!(!b.autosync);
        assert!(b.biometric_app_lock);
        // gate_idle is new (not in V4) — defaulted, not pulled.
        assert_eq!(b.gate_idle, GateIdle::default());
    }
}
