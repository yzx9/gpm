// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Migration `0002_config_scope_split` (RFC 0038).
//!
//! Copies the 5 app-scoped behavior prefs out of a pre-split `repo.json` into
//! `app.json`, producing the schema-2 shape ([`AppConfigV2`]) from the pre-split
//! schema-1 shape ([`AppConfigV1`]). The slimmed [`rustpass::RepoConfig`] drops
//! those fields on deserialize, so the legacy repo-scoped shape is read via
//! [`LegacyRepoConfig`]. `app.json` itself is read raw as [`AppConfigV1`] (which
//! still carries the deprecated `secure_screen` bool that `m0003` consumes and
//! `log_level` that `m0004` consumes — both absent from the latest runtime types).
//!
//! Idempotent (the engine gates on `schema_version`) and safe to call on every
//! startup and `app_unlock`.

use rustpass::{Error, LockMode};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::app_config::{
    SecureScreenMode, default_autosync_true, default_secure_screen, is_autosync_default, is_false,
};
use crate::identity::apply_security_caches_from;
use crate::migrations::MigrationOutcome;

/// The on-disk `app.json` shape at schema 1 (before the config-scope split):
/// just the app-shell prefs. Read raw by [`apply`] to recover `secure_screen`
/// (consumed by `m0003`) and `log_level` (consumed by `m0004`) plus the app-shell
/// prefs preserved into [`AppConfigV2`]. Deserialize-only — the transform
/// produces the next shape.
#[derive(Debug, Deserialize)]
pub(crate) struct AppConfigV1 {
    #[serde(default = "default_secure_screen")]
    pub(crate) secure_screen: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) locale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) theme_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) log_level: Option<String>,
}

/// Manual `Default` so `secure_screen` matches its serde default (`true`), not
/// the derived `bool` default (`false`). m0002 falls back to
/// `AppConfigV1::default()` when the on-disk V1 is unparseable (a wrong-type
/// field that `peek_schema_version` still parses); a derived `false` there would
/// make m0003 map it to `Off`, silently downgrading screen-capture protection
/// below the `Sensitive` default.
impl Default for AppConfigV1 {
    fn default() -> Self {
        Self {
            secure_screen: default_secure_screen(),
            locale: None,
            theme_mode: None,
            log_level: None,
        }
    }
}

/// Serde default for [`AppConfigV2::schema_version`] — `2`, the version this
/// migration produces. (Distinct from the shared `default_schema_version`, which
/// is `1` for the pre-split shapes.)
fn default_schema_v2() -> u32 {
    2
}

/// The on-disk `app.json` shape at schema 2: the app-shell prefs + the 5
/// behavior prefs copied in from `repo.json`. Still carries the deprecated
/// `secure_screen: bool` (consumed by `m0003`) and `log_level` (consumed by
/// `m0004`). `m0003` reads this raw as its source shape.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct AppConfigV2 {
    #[serde(default = "default_secure_screen")]
    pub(crate) secure_screen: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) secure_screen_mode: Option<SecureScreenMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) locale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) theme_mode: Option<String>,
    #[serde(default, skip_serializing_if = "LockMode::is_default")]
    pub(crate) lock_mode: LockMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) view_clear_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) clipboard_clear_secs: Option<u64>,
    #[serde(
        default = "default_autosync_true",
        skip_serializing_if = "is_autosync_default"
    )]
    pub(crate) autosync: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) biometric_app_lock: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) log_level: Option<String>,
    #[serde(default = "default_schema_v2")]
    pub(crate) schema_version: u32,
}

/// Manual `Default` so `secure_screen` and `autosync` match their serde
/// defaults (`true`), not the derived `bool` default (`false`). m0003 falls
/// back to `AppConfigV2::default()` when the on-disk V2 is unparseable (a
/// wrong-type field that `peek_schema_version` still parses); a derived `false`
/// would disagree with the serde default and produce a different lift than a
/// missing key.
impl Default for AppConfigV2 {
    fn default() -> Self {
        Self {
            secure_screen: default_secure_screen(),
            secure_screen_mode: None,
            locale: None,
            theme_mode: None,
            lock_mode: LockMode::default(),
            view_clear_secs: None,
            clipboard_clear_secs: None,
            autosync: default_autosync_true(),
            biometric_app_lock: false,
            log_level: None,
            schema_version: default_schema_v2(),
        }
    }
}

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

/// Copy the 5 app-scoped behavior prefs from a pre-split `repo.json` into
/// `app.json` (the source app-shell prefs come from the on-disk V1 shape),
/// write the schema-2 V2 shape, and re-seed the security caches + the `Store`'s
/// injected `autosync` from the just-written snapshot.
///
/// Outcomes:
/// - `SEAL_KEY_UNAVAILABLE` → [`MigrationOutcome::Pending`] (app-lock; the
///   sealed `repo.json` read fails until biometric injects the key; retried on
///   the next `app_unlock`).
/// - missing/unparseable `repo.json` (fresh install / post-reset / parse error)
///   → preserve the V1 app-shell prefs + default the 5 scope fields, bump
///   `schema_version`, `Done`.
/// - otherwise → copy, bump, write, re-seed, `Done`. A write failure is
///   propagated as `Err` so the engine leaves `schema_version` below target and
///   retries (never marks itself done without persisting).
pub(crate) async fn apply(state: &AppState, version: u32) -> Result<MigrationOutcome, Error> {
    // Read the pre-split app.json. The engine only runs us when peek saw a
    // schema < 2 (so the file exists); a full V1 parse failure is rare — fall
    // back to the V1 defaults so the app-shell prefs degrade rather than panic.
    let v1: AppConfigV1 = state.app_config.read_app_json_as().unwrap_or_else(|e| {
        log::warn!("0002_config_scope_split: app.json unparseable ({e}); using V1 defaults");
        AppConfigV1::default()
    });

    match state.store.load_repo_config_as::<LegacyRepoConfig>().await {
        Ok(legacy) => {
            // Preserve the app-shell prefs from V1; inject the 5 scope prefs
            // from the legacy repo.json.
            let v2 = AppConfigV2 {
                secure_screen: v1.secure_screen,
                secure_screen_mode: None,
                locale: v1.locale,
                theme_mode: v1.theme_mode,
                lock_mode: legacy.lock_mode,
                view_clear_secs: legacy.view_clear_secs,
                clipboard_clear_secs: legacy.clipboard_clear_secs,
                autosync: legacy.autosync,
                biometric_app_lock: legacy.biometric_app_lock,
                log_level: v1.log_level,
                schema_version: version,
            };
            write_and_seed(state, &v2).await?;
            Ok(MigrationOutcome::Done)
        }
        Err(e) if e.code == "SEAL_KEY_UNAVAILABLE" => {
            // App-lock: master key not available yet. Stay pending; the next
            // app_unlock (after biometric injects the key) retries.
            Ok(MigrationOutcome::Pending)
        }
        Err(e) => {
            // No repo.json (fresh install / post-reset) or a parse error — bump
            // schema_version so we don't retry forever; preserve V1's app-shell
            // prefs, default the 5 scope fields. A write failure is propagated
            // so the engine retries — never return Done without persisting the
            // bump, or the engine's `debug_assert_eq!` fires.
            log::warn!("0002_config_scope_split: nothing to copy ({e}); marking done");
            let v2 = AppConfigV2 {
                secure_screen: v1.secure_screen,
                secure_screen_mode: None,
                locale: v1.locale,
                theme_mode: v1.theme_mode,
                lock_mode: LockMode::default(),
                view_clear_secs: None,
                clipboard_clear_secs: None,
                autosync: default_autosync_true(),
                biometric_app_lock: false,
                log_level: v1.log_level,
                schema_version: version,
            };
            write_and_seed(state, &v2).await?;
            Ok(MigrationOutcome::Done)
        }
    }
}

/// Write the V2 snapshot to disk (raw — the cache is reloaded once at the end of
/// the chain), then seed the `AppState` security caches directly from it and
/// push `autosync` into the `Store`. Seeding from the snapshot (not the cache)
/// avoids a mid-chain reload: the raw write has not touched the in-memory cache,
/// and `apply_security_caches_from` takes the values explicitly.
async fn write_and_seed(state: &AppState, v2: &AppConfigV2) -> Result<(), Error> {
    state.app_config.write_app_json_raw(v2).await?;
    apply_security_caches_from(state, v2.lock_mode, v2.clipboard_clear_secs);
    state.store.set_autosync(v2.autosync);
    Ok(())
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
