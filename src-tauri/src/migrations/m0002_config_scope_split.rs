// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Migration `0002_config_scope_split` (RFC 0038).
//!
//! Copies the 5 app-scoped behavior prefs out of a pre-split `repo.json` into
//! `app.json`, then bumps `schema_version` so it never runs again. The slimmed
//! [`rustpass::RepoConfig`] drops those fields on deserialize, so the legacy
//! shape is read via [`LegacyRepoConfig`].
//!
//! Idempotent (the engine gates on `schema_version`) and safe to call on every
//! startup and `app_unlock`.

use rustpass::{Error, LockMode};
use serde::Deserialize;

use crate::AppState;
use crate::identity::apply_security_caches;
use crate::migrations::MigrationOutcome;

// NOTE: this migration carries the v1.0.0 removal TODO for the whole registry —
// see `migrations/mod.rs`.

/// The legacy `repo.json` shape for the 5 fields that moved to `AppConfig`.
/// Deserialize-only — used by [`apply`] to recover values the slimmed
/// `RepoConfig` drops on deserialize (serde ignores unknown fields, so this
/// reads a pre-split `repo.json` even though it also carries repo-scoped
/// fields). Defaults mirror the old `RepoConfig` so a file missing some keys
/// still parses.
#[derive(Debug, Deserialize)]
struct LegacyRepoConfig {
    #[serde(default)]
    lock_mode: LockMode,
    #[serde(default)]
    view_clear_secs: Option<u64>,
    #[serde(default)]
    clipboard_clear_secs: Option<u64>,
    #[serde(default = "default_autosync_true")]
    autosync: bool,
    #[serde(default)]
    biometric_app_lock: bool,
}

/// Serde default for `autosync` — `true` (matches the old `RepoConfig` default,
/// so a pre-split `repo.json` written before the toggle existed copies across
/// with autosync on).
fn default_autosync_true() -> bool {
    true
}

/// Copy the 5 app-scoped behavior prefs from a pre-split `repo.json` into
/// `app.json` (mutating the cached `AppConfig`, preserving the app-scoped prefs
/// already there), bump `schema_version`, save, and re-seed the security caches
/// + the `Store`'s injected `autosync`.
///
/// Outcomes:
/// - `SEAL_KEY_UNAVAILABLE` → [`MigrationOutcome::Pending`] (app-lock; the
///   sealed `repo.json` read fails until biometric injects the key; retried on
///   the next `app_unlock`).
/// - missing/unparseable `repo.json` (fresh install / post-reset / parse error)
///   → bump `schema_version` and return `Done` with nothing copied.
/// - otherwise → copy, bump, save, re-seed, `Done`. A save failure is
///   propagated as `Err` so the engine leaves `schema_version` below target and
///   retries on the next run (never marks itself done without persisting).
pub(crate) async fn apply(state: &AppState, version: u32) -> Result<MigrationOutcome, Error> {
    match state.store.load_repo_config_as::<LegacyRepoConfig>().await {
        Ok(legacy) => {
            // Mutate the cached AppConfig — never build a fresh one (would wipe
            // the app-scoped prefs). Preserve everything but the 5 fields + the
            // version.
            let mut cfg = state.app_config.get();
            cfg.lock_mode = legacy.lock_mode;
            cfg.view_clear_secs = legacy.view_clear_secs;
            cfg.clipboard_clear_secs = legacy.clipboard_clear_secs;
            cfg.autosync = legacy.autosync;
            cfg.biometric_app_lock = legacy.biometric_app_lock;
            cfg.schema_version = version;
            // Propagate a save failure as Err so the engine leaves schema below
            // target and retries — never mark Done without persisting.
            state.app_config.save(&cfg).await?;
            // Re-seed every cache that reads these values.
            apply_security_caches(state);
            state.store.set_autosync(cfg.autosync);
            Ok(MigrationOutcome::Done)
        }
        Err(e) if e.code == "SEAL_KEY_UNAVAILABLE" => {
            // App-lock: master key not available yet. Stay pending; the next
            // app_unlock (after biometric injects the key) retries.
            Ok(MigrationOutcome::Pending)
        }
        Err(e) => {
            // No repo.json (fresh install / post-reset) or a parse error — bump
            // schema_version so we don't retry forever; nothing to copy. A save
            // failure is propagated so the engine retries (matching the copy
            // branch above) — never return Done without persisting the bump, or
            // the engine's `debug_assert_eq!(schema_version, version)` fires.
            log::warn!("0002_config_scope_split: nothing to copy ({e}); marking done");
            let mut cfg = state.app_config.get();
            cfg.schema_version = version;
            state.app_config.save(&cfg).await?;
            Ok(MigrationOutcome::Done)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The legacy reader must recover non-default values from a pre-split
    /// `repo.json` — including the `LockMode::Idle(N)` serde shape — even though
    /// the slimmed `RepoConfig` drops them. This is the core of the compat
    /// regression: without a working legacy reader the migration silently no-ops.
    #[test]
    fn legacy_repo_config_parses_old_shape_with_non_defaults() {
        let json = br#"{
            "url":"https://x/repo.git","pat":"t","local_path":"/p",
            "commit_user_name":"Alice",
            "lock_mode":{"idle":300},
            "view_clear_secs":0,
            "clipboard_clear_secs":180,
            "autosync":false,
            "biometric_app_lock":true
        }"#;
        let legacy: LegacyRepoConfig = serde_json::from_slice(json).unwrap();
        assert_eq!(legacy.lock_mode, LockMode::Idle(300));
        assert_eq!(legacy.view_clear_secs, Some(0));
        assert_eq!(legacy.clipboard_clear_secs, Some(180));
        assert!(!legacy.autosync);
        assert!(legacy.biometric_app_lock);
    }

    /// A `repo.json` that never set the behavior prefs (or a fresh slimmed one)
    /// parses with the defaults — so the migration copies defaults, not garbage.
    #[test]
    fn legacy_repo_config_defaults_when_fields_absent() {
        let json = br#"{"url":"u","local_path":"/p"}"#;
        let legacy: LegacyRepoConfig = serde_json::from_slice(json).unwrap();
        assert_eq!(legacy.lock_mode, LockMode::Immediate);
        assert_eq!(legacy.view_clear_secs, None);
        assert_eq!(legacy.clipboard_clear_secs, None);
        assert!(legacy.autosync);
        assert!(!legacy.biometric_app_lock);
    }
}
