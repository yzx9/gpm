// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Migration `0006_gate_idle_timeout`.
//!
//! Sets the new `gate_idle` behavior field to `Off` on existing configs, so an
//! upgrading `AppLock` user does not suddenly start getting a 5-min idle lock they
//! never asked for (new installs skip this — their `BehaviorConfig::default`
//! starts at `After(300)`, and `schema_version` starts at the registry target).
//!
//! `gate_idle` lives in the sealed behavior slot (`app.json`), so this migration
//! unseals → modifies → reseals it, which needs the master key. Under the
//! app-launch biometric lock the key is wiped at cold start (injected only after
//! the biometric prompt), so the migration defers (`Pending`) and the next
//! `app_unlock` retries from the top — mirrors `m0005_split_app_json`'s app-lock
//! resume pattern.
//!
//! It loads the behavior fresh from disk (`Store::load_app_behavior`) rather
//! than the in-memory cache: `run_app_migrations` runs BEFORE
//! `reload_behavior`, so the cache is still the default at this point, and
//! reading it would clobber the user's real `lock_mode`/`autosync`/etc. with
//! defaults.
//!
//! Idempotent (gated on `schema_version`, which lives in `pref.json`) and safe
//! to call on every startup and `app_unlock`.

use std::sync::atomic::Ordering;

use rustpass::Error;

use crate::AppState;
use crate::app_config::{BehaviorConfig, GateIdle};
use crate::migrations::MigrationOutcome;

/// Set `behavior.gate_idle = Off` on existing configs, bumping `schema_version`
/// to 6. See the module docs for the app-lock resume semantics.
///
/// Outcomes:
/// - `schema_version >= 6` on entry → `Done` (idempotent re-entry; the registry
///   also gates on this, but double-check).
/// - `app_lock_enabled && !has_master_key()` → `Pending` (the master key is
///   wiped at cold start under the app-launch gate; the next `app_unlock`
///   retries from the top).
/// - `SealKeyUnavailable` from the unseal/reseal → `Pending` (defensive; the
///   guard should have caught it).
/// - missing/unreadable behavior slot → bump `pref.json` schema to 6, `Done`
///   (nothing to migrate; mirrors `m0005` resilience — never brick startup).
/// - otherwise → load behavior, set `gate_idle = Off`, reseal, bump schema to 6.
pub(crate) async fn apply(state: &AppState, version: u32) -> Result<MigrationOutcome, Error> {
    // The engine gates on `peek_schema_version` (a raw disk read), so by here
    // schema < `version` — no per-step idempotency check (mirrors `m0005`). The
    // in-memory pref cache is intentionally stale mid-chain (migrations write
    // raw, no per-step swap), so reading it here would be wrong.

    // 1. App-lock guard: the behavior slot is sealed, so unseal/reseal needs the
    //    master key. Under the app-launch gate the key is wiped at cold start;
    //    defer so the next app_unlock retries from the top.
    if state.app_lock_enabled.load(Ordering::SeqCst) && !state.store.has_master_key() {
        return Ok(MigrationOutcome::Pending);
    }

    // 2. Load + unseal the current behavior fresh from disk (NOT the cache,
    //    which is still default here — see the module docs).
    let mut behavior = match state.store.load_app_behavior().await {
        Ok(bytes) => match serde_json::from_slice::<BehaviorConfig>(&bytes) {
            Ok(b) => b,
            Err(e) => {
                log::warn!("0006_gate_idle_timeout: behavior unparseable ({e}); marking done");
                return bump_pref_schema_to(state, version)
                    .await
                    .map(|()| MigrationOutcome::Done);
            }
        },
        Err(e) if e.code == "SEAL_KEY_UNAVAILABLE" => {
            // Defensive: the guard above should have caught this, but a race
            // with app_lock mid-write surfaces here. Defer to the next app_unlock.
            return Ok(MigrationOutcome::Pending);
        }
        Err(e) if e.code == "NO_IDENTITY" => {
            // No sealed behavior slot yet — nothing to migrate. Mark done.
            return bump_pref_schema_to(state, version)
                .await
                .map(|()| MigrationOutcome::Done);
        }
        Err(e) => {
            log::warn!("0006_gate_idle_timeout: behavior load failed ({e}); marking done");
            return bump_pref_schema_to(state, version)
                .await
                .map(|()| MigrationOutcome::Done);
        }
    };

    // 3. Pin gate_idle to Off for this existing config, then reseal.
    behavior.gate_idle = GateIdle::Off;
    if let Err(e) = state.app_config.save_behavior(&behavior).await {
        if e.code == "SEAL_KEY_UNAVAILABLE" {
            return Ok(MigrationOutcome::Pending);
        }
        return Err(e);
    }

    // 4. Bump pref schema to target. A failure here surfaces as Err so the
    //    engine retries (schema stays at the preserved value; the behavior
    //    write above is idempotent because it overwrites).
    bump_pref_schema_to(state, version).await?;
    Ok(MigrationOutcome::Done)
}

/// Read the current pref cache, bump its `schema_version` to `version`, and
/// persist atomically. Mirrors `m0005_split_app_json::bump_pref_schema_to`.
async fn bump_pref_schema_to(state: &AppState, version: u32) -> Result<(), Error> {
    let mut pref = state.app_config.get_pref();
    pref.schema_version = version;
    state.app_config.save_pref(&pref).await
}

#[cfg(test)]
mod tests {
    // The migration's behavior is exercised end-to-end via the migration
    // registry tests (schema 5 → 6, gate_idle Off, idempotent re-run). The
    // GateIdle serde/default/clamp unit tests live in `app_config::tests`.
}
