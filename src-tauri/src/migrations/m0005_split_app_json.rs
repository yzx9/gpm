// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Migration `0005_split_app_json`.
//!
//! Splits the single plaintext `app.json` (the legacy [`AppConfig`] single-file
//! shape that `m0002`/`m0003`/`m0004_verbose_from_debug` last wrote) into the
//! post-split pair:
//! - **`pref.json`** (plaintext) — display prefs (`locale`, `theme_mode`,
//!   `log_level`, `verbose_until`, the deprecated `secure_screen` bool, and
//!   `schema_version`).
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

use std::sync::atomic::Ordering;

use rustpass::Error;

use crate::AppState;
use crate::app_config::{BehaviorConfig, PrefConfig};
use crate::migrations::MigrationOutcome;

// NOTE: this migration carries the v1.0.0 removal TODO for the whole registry —
// see `migrations/mod.rs`.

/// Split the legacy plaintext `app.json` into `pref.json` + sealed `app.json`,
/// bumping `schema_version` to 5. See the module docs for the app-lock resume
/// semantics.
///
/// Outcomes:
/// - `schema_version >= 5` on entry → `Done` (idempotent re-entry; the registry
///   also gates on this, but double-check).
/// - missing `app.json` (fresh install / post-reset) → write `pref.json` with
///   `schema_version` bumped to 5, return `Done` (no behavior to seal).
/// - `app.json` already an envelope (half-migrated recovery: a prior run sealed
///   the behavior half but crashed before bumping the schema) → bump `pref.json`
///   schema to 5, return `Done`.
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
    // 1. Idempotent re-entry (the registry gates on this too).
    if state.app_config.get_pref().schema_version >= version {
        return Ok(MigrationOutcome::Done);
    }

    // 2. Read raw app.json bytes from disk.
    let app_json_path = state.app_config.app_json_path();
    let bytes = match std::fs::read(app_json_path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
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
            log::warn!("0004_split_app_json: app.json unreadable ({e}); marking done");
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

    // 5. Plaintext legacy: parse as the AppConfig single-file shape, then split.
    let legacy: crate::app_config::AppConfig = match serde_json::from_slice(&bytes) {
        Ok(c) => c,
        Err(e) => {
            // Unparseable as legacy: warn + mark done. The file is in an
            // unknown shape; rather than brick startup, treat it as migrated.
            log::warn!(
                "0004_split_app_json: app.json unparseable as legacy AppConfig ({e}); marking done"
            );
            return bump_pref_schema_to(state, version)
                .await
                .map(|()| MigrationOutcome::Done);
        }
    };

    // Write the display half to pref.json FIRST, but ONLY on the first run
    // (pref.json absent). Once pref.json exists the display half is already
    // split and pref.json is authoritative for it — re-deriving display prefs
    // from app.json would clobber the user's locale/theme/log on desktop,
    // where a half-migrated app.json (step 7 landed, step 8 below crashed) is
    // a plaintext `BehaviorConfig` that parses as a degenerate `AppConfig`
    // with defaulted display fields + `schema_version: 1`. PRESERVE
    // schema_version (do NOT bump yet) — the schema advances only after the
    // sealed write succeeds, so a Pending resume re-enters cleanly.
    if !state.app_config.pref_json_exists() {
        state
            .app_config
            .save_pref(&PrefConfig::from_legacy(&legacy))
            .await?;
    }

    // 6. App-lock guard: master key not yet injected. Defer — the next
    //    app_unlock retries from the top (after biometric injects the key).
    if state.app_lock_enabled.load(Ordering::SeqCst) && !state.store.has_master_key() {
        return Ok(MigrationOutcome::Pending);
    }

    // 7. Build behavior from the legacy fields and seal it into app.json.
    let behavior = BehaviorConfig::from_legacy(&legacy);
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

    /// `BehaviorConfig::from_legacy` is the load-bearing extraction for the
    /// split — it must pull all six behavior fields off the parsed legacy
    /// `AppConfig`. The cross-module `behavior_config_from_legacy_preserves_behavior_fields`
    /// test in `app_config::tests` already pins this; this test exists as a
    /// local mirror so a refactor that breaks the extraction fails inside the
    /// migration's own tests (closer to the bug).
    #[test]
    fn from_legacy_pulls_all_six_behavior_fields() {
        use crate::app_config::{AppConfig, SecureScreenMode};
        let app = AppConfig {
            secure_screen_mode: Some(SecureScreenMode::Always),
            lock_mode: rustpass::LockMode::Idle(120),
            view_clear_secs: Some(0),
            clipboard_clear_secs: Some(180),
            autosync: false,
            biometric_app_lock: true,
            ..Default::default()
        };
        let b = BehaviorConfig::from_legacy(&app);
        assert_eq!(b.secure_screen_mode, Some(SecureScreenMode::Always));
        assert_eq!(b.lock_mode, rustpass::LockMode::Idle(120));
        assert_eq!(b.view_clear_secs, Some(0));
        assert_eq!(b.clipboard_clear_secs, Some(180));
        assert!(!b.autosync);
        assert!(b.biometric_app_lock);
    }
}
