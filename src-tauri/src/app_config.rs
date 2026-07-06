// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! App-shell configuration that must persist before any repo is set up.
//!
//! Today this holds the screen-capture master toggle ([`AppConfig::secure_screen`])
//! and the display-language preference ([`AppConfig::locale`]). Both live at
//! `app.json` in the config directory — distinct from `repo.json`, which is
//! repo-scoped and (on Android) sealed at rest. `app.json` is a plaintext UI
//! preference (no secret); encrypting it would be theater and would couple this
//! app-shell module to the `rustpass` store layer.
//!
//! `app.json` intentionally survives `reset_config` (which wipes the repo dir,
//! `identity`, and `repo.json`): these are device-level preferences, not repo
//! data, so re-setting up the repo should not reset the user's screen-capture or
//! language choice.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rustpass::{Error, ErrorCode};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::AppState;

/// File name of the app-level config, inside the config directory.
const APP_CONFIG_FILE: &str = "app.json";

/// Locales the app ships translations for. An explicit preference must be one
/// of these; anything else degrades to the system-locale resolution.
const SUPPORTED_LOCALES: [&str; 2] = ["en", "zh-CN"];

/// App-level (non-repo) preferences. Plaintext on disk — no secrets, only UI
/// toggles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AppConfig {
    /// Master toggle for per-page screen-capture protection. Default ON
    /// (`true`): sensitive routes block screenshots/recording. When `false`,
    /// no page is ever secured (the user explicitly allowed capture).
    #[serde(default = "default_secure_screen")]
    pub(crate) secure_screen: bool,
    /// Display-language override. `None` (the default) means "track the system
    /// language" — the backend resolves the system locale at boot. `Some("en")`
    /// / `Some("zh-CN")` pins the locale explicitly. `skip_serializing_if`
    /// keeps existing `app.json` files (which predate this field) byte-identical
    /// on round-trip, so adding the field is non-breaking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) locale: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            secure_screen: default_secure_screen(),
            locale: None,
        }
    }
}

/// Serde default for [`AppConfig::secure_screen`] — `true` (secure by default).
fn default_secure_screen() -> bool {
    true
}

/// True if `code` is one of [`SUPPORTED_LOCALES`].
fn is_supported_locale(code: &str) -> bool {
    SUPPORTED_LOCALES.contains(&code)
}

/// Reject an unsupported explicit locale code. `None` (track system) is always
/// valid; `Some(code)` must be in [`SUPPORTED_LOCALES`].
fn validate_locale(locale: Option<&str>) -> Result<(), Error> {
    if let Some(code) = locale
        && !is_supported_locale(code)
    {
        return Err(Error::new(
            ErrorCode::ConfigError,
            format!("Unsupported locale code '{code}'"),
        ));
    }
    Ok(())
}

/// Persistent app-shell config, owned by [`AppState`]. The on-disk file is read
/// once synchronously at construction; the in-memory cache is authoritative
/// thereafter. The [`Mutex`] guard is never held across an `.await`.
#[derive(Debug)]
pub(crate) struct AppConfigStore {
    path: PathBuf,
    cache: Mutex<AppConfig>,
}

impl AppConfigStore {
    /// Load the app config from `config_dir/app.json`, falling back to the
    /// default (secure ON) if the file is missing or corrupt.
    #[must_use]
    pub(crate) fn new(config_dir: &Path) -> Self {
        let path = config_dir.join(APP_CONFIG_FILE);
        let cache = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<AppConfig>(&s).ok())
            .unwrap_or_default();
        Self {
            path,
            cache: Mutex::new(cache),
        }
    }

    /// Snapshot the cached config.
    pub(crate) fn get(&self) -> AppConfig {
        self.cache.lock().expect("app config lock poisoned").clone()
    }

    /// Persist `cfg` atomically (temp + rename, mirroring
    /// `rustpass::config::save_atomic`) and update the cache.
    ///
    /// The `Mutex` is held only for the final cache swap — never across the
    /// `tokio::fs` `.await` points (the write/rename complete before the guard
    /// is taken), so there is no await-held-lock deadlock risk.
    async fn save(&self, cfg: &AppConfig) -> Result<(), Error> {
        let json = serde_json::to_string_pretty(cfg)?;
        let tmp = self.path.with_extension("tmp");
        tokio::fs::write(&tmp, json).await?;
        tokio::fs::rename(&tmp, &self.path).await?;
        *self.cache.lock().expect("app config lock poisoned") = cfg.clone();
        Ok(())
    }
}

/// Whether the screen-secure plugin is available on this platform. Compile-time
/// `true` on Android (where `FLAG_SECURE` exists), `false` everywhere else.
///
/// The frontend caches this so it never invokes the plugin command on a
/// platform where it does not exist. This is explicit availability — not
/// inferred from invoke success — so a broken plugin on Android is never
/// mistaken for desktop (which would fail open).
#[tauri::command]
pub(crate) fn screen_secure_available() -> bool {
    cfg!(target_os = "android")
}

/// Read the app config (the master toggle).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn get_app_config(state: State<'_, AppState>) -> AppConfig {
    state.app_config.get()
}

/// Set the screen-capture master toggle and persist it. Returns the updated
/// config; the frontend re-applies the current route's secure state on receipt.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn set_secure_screen(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<AppConfig, Error> {
    let mut cfg = state.app_config.get();
    cfg.secure_screen = enabled;
    state.app_config.save(&cfg).await?;
    Ok(cfg)
}

/// Set the display-language preference and persist it. `locale: null` clears
/// the override (track system); `"en"` / `"zh-CN"` pin it. Returns the updated
/// config. The frontend re-applies the locale on receipt.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn set_locale_pref(
    state: State<'_, AppState>,
    locale: Option<String>,
) -> Result<AppConfig, Error> {
    validate_locale(locale.as_deref())?;
    let mut cfg = state.app_config.get();
    cfg.locale = locale;
    state.app_config.save(&cfg).await?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    fn store_at(dir: &Path) -> AppConfigStore {
        AppConfigStore::new(dir)
    }

    #[tokio::test]
    async fn missing_file_defaults_secure_on() {
        let dir = tempdir().expect("tempdir");
        assert!(store_at(dir.path()).get().secure_screen);
    }

    #[tokio::test]
    async fn corrupt_file_defaults_secure_on() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join(APP_CONFIG_FILE), "{not json").unwrap();
        assert!(store_at(dir.path()).get().secure_screen);
    }

    #[tokio::test]
    async fn roundtrip_persists_toggle() {
        let dir = tempdir().expect("tempdir");
        let store = store_at(dir.path());
        assert!(store.get().secure_screen, "default must be ON");

        // Flip OFF, persist, and reload from disk to confirm it landed.
        store
            .save(&AppConfig {
                secure_screen: false,
                locale: None,
            })
            .await
            .unwrap();
        assert!(!store.get().secure_screen);
        assert!(
            !store_at(dir.path()).get().secure_screen,
            "reload must see the persisted OFF"
        );

        // Flip back ON and reload.
        store_at(dir.path())
            .save(&AppConfig {
                secure_screen: true,
                locale: None,
            })
            .await
            .unwrap();
        assert!(store_at(dir.path()).get().secure_screen);
    }

    #[test]
    fn default_locale_is_none() {
        assert!(AppConfig::default().locale.is_none());
    }

    #[tokio::test]
    async fn locale_roundtrips_through_save() {
        let dir = tempdir().expect("tempdir");
        let store = store_at(dir.path());
        store
            .save(&AppConfig {
                secure_screen: true,
                locale: Some("zh-CN".to_string()),
            })
            .await
            .unwrap();
        let reloaded = store_at(dir.path()).get();
        assert_eq!(reloaded.locale.as_deref(), Some("zh-CN"));
    }

    #[tokio::test]
    async fn locale_omitted_on_disk_when_none() {
        // skip_serializing_if keeps the field out of app.json when it is None,
        // so existing files stay byte-identical and don't carry a null.
        let dir = tempdir().expect("tempdir");
        let store = store_at(dir.path());
        store
            .save(&AppConfig {
                secure_screen: true,
                locale: None,
            })
            .await
            .unwrap();
        let on_disk = std::fs::read_to_string(dir.path().join(APP_CONFIG_FILE)).unwrap();
        assert!(
            !on_disk.contains("locale"),
            "locale key must be absent when None; got: {on_disk}"
        );
    }

    #[test]
    fn existing_app_json_without_locale_loads() {
        // An app.json written before the locale field existed must still parse,
        // with locale defaulting to None (backward compatibility).
        let dir = tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join(APP_CONFIG_FILE),
            r#"{"secure_screen":true}"#,
        )
        .unwrap();
        let cfg = store_at(dir.path()).get();
        assert!(cfg.secure_screen);
        assert!(cfg.locale.is_none());
    }

    #[test]
    fn validate_locale_accepts_supported_and_none() {
        assert!(validate_locale(None).is_ok());
        assert!(validate_locale(Some("en")).is_ok());
        assert!(validate_locale(Some("zh-CN")).is_ok());
    }

    #[test]
    fn validate_locale_rejects_unknown() {
        let err = validate_locale(Some("zh-TW")).unwrap_err();
        assert_eq!(err.code, "CONFIG_ERROR");
        assert!(err.message.contains("zh-TW"));
        assert!(validate_locale(Some("fr")).is_err());
    }
}
