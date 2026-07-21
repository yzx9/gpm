// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! App-shell configuration that must persist before any repo is set up, and
//! survive a repository re-setup. See RFC 0038 for the full model.
//!
//! # The three persistence tiers
//!
//! gpm persists state across three tiers; this module owns the third:
//!
//! 1. **Git** — the cloned gopass repository of age-encrypted secrets, version-
//!    controlled and synced via `git pull`/`push`. The only tier that leaves the
//!    device. (The on-disk clone lives under the path `repo.json` points at.)
//! 2. **Sealed files** — `repo.json` (repo-scoped config) and `identity`, sealed
//!    at rest with AEAD on Android, plaintext on desktop. Owned by `rustpass`.
//!    See [`rustpass::config::Config`].
//! 3. **Plaintext files** — **`app.json` (this module)**, always plaintext.
//!
//! `app.json` is **plaintext on disk**, and this is forced, not a shortcut:
//! `locale` must be readable before unlock (first-paint injection + the app-lock
//! biometric screen), and sealing `app.json` would make it unreadable at setup
//! when app-lock is on. None of these prefs are confidential, and the local
//! write attacker is out of scope per the threat model, so plaintext is
//! consistent. (The `WebView`'s `localStorage` is explicitly not a tier — it may
//! be cleared by the system, so it is never authoritative for settings.)
//!
//! # What lives here
//!
//! The screen-capture master toggle ([`AppConfig::secure_screen`]), the
//! display-language preference ([`AppConfig::locale`]), and the behavior prefs
//! that moved here from `RepoConfig` in the RFC 0038 scope split: `lock_mode`,
//! the view/clipboard clear timers, `autosync`, and `biometric_app_lock`. All
//! are application-scoped (survive a repository re-setup) and non-confidential.
//!
//! `app.json` intentionally survives `reset_config` (which wipes the repo dir,
//! `identity`, `repo.json`, and the `app_id_pass` slot): these are device-level
//! preferences, not repo data, so re-setting up the repo does not reset the
//! user's language, timers, autosync, or app-lock choice.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rustpass::config::DEFAULT_CLIPBOARD_CLEAR_SECS;
use rustpass::{Error, ErrorCode, LockMode, clamp_lock_mode, normalize_clear_secs};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::AppState;

/// File name of the app-level config, inside the config directory.
const APP_CONFIG_FILE: &str = "app.json";

/// Locales the app ships translations for. An explicit preference must be one
/// of these; anything else degrades to the system-locale resolution.
const SUPPORTED_LOCALES: [&str; 2] = ["en", "zh-CN"];

/// The locale used when no preference is set and the system locale is neither
/// English nor Chinese — keeps an unsupported system from rendering blank keys.
const DEFAULT_LOCALE: &str = "en";

/// App-level (non-repo) preferences. Plaintext on disk — no secrets, only UI
/// behavior prefs. Plaintext (not sealed) is forced: `locale` must be readable
/// before unlock for the first-paint injection + app-lock biometric screen, and
/// sealing `app.json` would make it unreadable at setup when app-lock is on. The
/// other prefs ride along (none are confidential; the local write attacker is
/// out of scope per the threat model).
///
/// The behavior prefs (`lock_mode`, clear timers, `autosync`,
/// `biometric_app_lock`) moved here from `RepoConfig` (the RFC 0038 scope split)
/// so they survive a repository re-setup instead of being wiped with repo data.
/// Three-state screen-capture protection mode. Serialized kebab-case as
/// `"off"` / `"sensitive"` / `"always"`. [`SecureScreenMode::Unknown`] is a
/// forward-compatibility sink (`#[serde(other)]`): a value written by a newer
/// build deserializes to `Unknown` instead of failing `AppConfig` parsing
/// (which would wipe the whole config back to defaults). The frontend treats
/// `None` and `Unknown` as the sensitive default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SecureScreenMode {
    Off,
    Sensitive,
    Always,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AppConfig {
    /// **Deprecated** boolean master toggle, kept only so migration
    /// `0003_secure_screen_mode` can recover the pre-three-state value; removed
    /// at v1.0.0 with the rest of the migration registry. Default ON (`true`).
    #[serde(default = "default_secure_screen")]
    pub(crate) secure_screen: bool,
    /// Three-state screen-capture protection. `None` (the default) ⇒
    /// `Sensitive` (the frontend resolves `None`/`Unknown` to `Sensitive`):
    /// sensitive routes + nav transitions + the unlock overlay block capture,
    /// the entry list / history stay capturable. `Off` ⇒ no screen is ever
    /// secured (the user explicitly allowed capture, including the unlock
    /// overlay). `Always` ⇒ every screen is secured at all times.
    /// `skip_serializing_if` keeps the field out of `app.json` while `None`, so
    /// a default config stays byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) secure_screen_mode: Option<SecureScreenMode>,
    /// Display-language override. `None` (the default) means "track the system
    /// language" — the backend resolves the system locale at boot. `Some("en")`
    /// / `Some("zh-CN")` pins the locale explicitly. `skip_serializing_if`
    /// keeps existing `app.json` files (which predate this field) byte-identical
    /// on round-trip, so adding the field is non-breaking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) locale: Option<String>,
    /// Color-scheme (light/dark) override. `None` (the default) means "track the
    /// system preference" — the frontend's `prefers-color-scheme` CSS media
    /// query governs, zero-JS and zero-flash. `Some("light")` / `Some("dark")`
    /// pins it via a `<html data-theme>` attribute the frontend sets after
    /// reading this. Plaintext here (not sealed) for the same reason as
    /// `locale`: it must render before unlock and survive `reset_config`.
    /// `skip_serializing_if` keeps existing `app.json` files byte-identical on
    /// round-trip, so adding the field is non-breaking (mirrors `locale`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) theme_mode: Option<String>,
    /// How the app auto-locks the identity cache. Skipped from serialization
    /// when default (`Immediate`), so an uncustomized config is byte-identical
    /// to one written before this field moved here.
    #[serde(default, skip_serializing_if = "LockMode::is_default")]
    pub(crate) lock_mode: LockMode,
    /// Seconds a revealed password stays in the DOM before auto-clear.
    /// `None` ⇒ [`DEFAULT_VIEW_CLEAR_SECS`]; `Some(0)` ⇒ never auto-clear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) view_clear_secs: Option<u64>,
    /// Seconds the clipboard holds a copied password before auto-clear.
    /// `None` ⇒ [`DEFAULT_CLIPBOARD_CLEAR_SECS`]; `Some(0)` ⇒ never auto-clear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) clipboard_clear_secs: Option<u64>,
    /// Whether each save wraps in a pull→write→push (gopass-style per-command
    /// sync). Default `true`; omitted from serialization while `true`.
    #[serde(
        default = "default_autosync_true",
        skip_serializing_if = "is_autosync_default"
    )]
    pub(crate) autosync: bool,
    /// Persisted intent for the app-launch biometric gate. **Write-only** — the
    /// Settings toggle and the runtime gate read the Keystore probe via
    /// `get_app_lock_state`, not this flag; it exists only as a persisted record
    /// mirroring the old `RepoConfig` field. Skipped when `false`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) biometric_app_lock: bool,
    /// Persisted-schema version for one-shot migrations. `1` is the pre-split
    /// shape; the `migrations` registry bumps it as each step runs (target:
    /// `migrations::APP_CONFIG_SCHEMA_VERSION`).
    #[serde(default = "default_schema_version")]
    pub(crate) schema_version: u32,
    /// Persisted diagnostics log level (`None` ⇒ default `Info`). One of
    /// `"error"`, `"warn"`, `"info"`, `"debug"`. Applied at startup via
    /// `log::set_max_level` (see `logging`/`lib::init_state`) and on the
    /// `set_log_level` command. Plaintext here (not sealed) so it is readable
    /// before unlock and survives `reset_config` — same rationale as `locale`.
    /// Omitted from `app.json` while `None` (the default) so existing files stay
    /// byte-identical on round-trip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) log_level: Option<String>,
}

impl Default for AppConfig {
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
            // A brand-new config starts at the current target so it skips the
            // legacy no-op migrations. (The serde missing-key default below
            // stays at 1 so a pre-split app.json still runs the registry.)
            schema_version: crate::migrations::APP_CONFIG_SCHEMA_VERSION,
            log_level: None,
        }
    }
}

/// Serde default for [`AppConfig::secure_screen`] — `true` (secure by default).
fn default_secure_screen() -> bool {
    true
}

/// Serde default for [`AppConfig::autosync`] — `true` (gopass-style per-save
/// pull→write→push on by default).
fn default_autosync_true() -> bool {
    true
}

/// `true` (the default) so `autosync` is omitted from `app.json` while on — a
/// user who never toggles it sees no change to the file's shape.
#[allow(clippy::trivially_copy_pass_by_ref)] // serde's skip_serializing_if needs `fn(&T)`
fn is_autosync_default(autosync: &bool) -> bool {
    *autosync
}

/// `false` (the default) so `biometric_app_lock` is omitted from `app.json`
/// when off.
#[allow(clippy::trivially_copy_pass_by_ref)] // serde's skip_serializing_if needs `fn(&T)`
fn is_false(b: &bool) -> bool {
    !*b
}

/// Serde default for [`AppConfig::schema_version`] when the key is missing —
/// `1`, the version before the config-scope migration existed. A pre-split
/// `app.json` that omits the key must still run the registry (otherwise it
/// would skip straight to the target and silently lose the scope split + the
/// bool→mode conversion), so this stays at `1`. A brand-new install is built
/// via [`AppConfig::default`], which starts at `APP_CONFIG_SCHEMA_VERSION`
/// instead (skipping the legacy no-op steps) — the two differ on purpose.
fn default_schema_version() -> u32 {
    1
}

impl AppConfig {
    /// Effective clipboard auto-clear seconds: `None` resolves to
    /// [`DEFAULT_CLIPBOARD_CLEAR_SECS`], otherwise the configured value.
    #[must_use]
    pub(crate) fn clipboard_clear_secs_effective(&self) -> u64 {
        self.clipboard_clear_secs
            .unwrap_or(DEFAULT_CLIPBOARD_CLEAR_SECS)
    }
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

/// Color-scheme overrides the settings page exposes. `None` (track system) is
/// always valid and is not listed here; an explicit `Some` must be one of these.
/// Do NOT add `"system"` here: the frontend sends `null` for "track system"
/// (never the string), and persisting `Some("system")` would break the
/// byte-identical-on-default invariant `locale`/`log_level` rely on.
const SUPPORTED_THEME_MODES: [&str; 2] = ["light", "dark"];

/// Reject an unsupported explicit theme mode. `None` (track system) is always
/// valid; `Some(mode)` must be in [`SUPPORTED_THEME_MODES`]. Mirrors
/// `validate_locale` / `validate_log_level`.
fn validate_theme_mode(mode: Option<&str>) -> Result<(), Error> {
    if let Some(m) = mode
        && !SUPPORTED_THEME_MODES.contains(&m)
    {
        return Err(Error::new(
            ErrorCode::ConfigError,
            format!("Unsupported theme mode '{m}'"),
        ));
    }
    Ok(())
}

/// Log levels the diagnostics viewer exposes (lowercase, matching `log::Level`).
/// `trace`/`off` are intentionally excluded — too noisy / disables logging.
const LOG_LEVELS: [&str; 4] = ["error", "warn", "info", "debug"];

/// Reject an unsupported log level. `None` (use the default `Info`) is always
/// valid; `Some(level)` must be one of [`LOG_LEVELS`]. Mirrors `validate_locale`.
fn validate_log_level(level: Option<&str>) -> Result<(), Error> {
    if let Some(lvl) = level
        && !LOG_LEVELS.contains(&lvl)
    {
        return Err(Error::new(
            ErrorCode::ConfigError,
            format!("Unsupported log level '{lvl}'"),
        ));
    }
    Ok(())
}

/// Map a BCP-47 system-locale tag (from `sys_locale::get_locale`) to one of the
/// supported locale codes. Chinese variants collapse to `zh-CN`, English
/// variants to `en`, anything else (or `None`) falls back to [`DEFAULT_LOCALE`].
fn normalize_system_locale(raw: Option<&str>) -> String {
    match raw {
        Some(s) if s.to_ascii_lowercase().starts_with("zh") => "zh-CN".to_string(),
        Some(s) if s.to_ascii_lowercase().starts_with("en") => "en".to_string(),
        _ => DEFAULT_LOCALE.to_string(),
    }
}

/// The locale to bake into the `WebView` initialization script.
///
/// This runs at Tauri `Builder` time, before the `App` exists — so on Android
/// the config directory (and thus `app.json`) is not yet readable (it is only
/// resolvable through the running app's mobile-plugin IPC). The system locale
/// is readable this early, though (`sys_locale` reads it via libc, no app
/// required, on every platform), so the inject carries the "track system"
/// resolution. This is exactly correct for users who haven't pinned a language
/// (the default, and the first-launch case), and the boot `resolved_locale`
/// IPC corrects it within one frame for users who have.
pub(crate) fn init_script_locale() -> String {
    normalize_system_locale(sys_locale::get_locale().as_deref())
}

/// The full JavaScript snippet that bakes the boot locale into the `WebView` as
/// `window.__GPM_LOCALE__` before the page's own scripts run. Registered on the
/// Tauri `Builder` (`append_invoke_initialization_script`) so it applies to
/// every webview on every platform, riding the same channel that sets up
/// `__TAURI_INTERNALS__`.
pub(crate) fn locale_init_script() -> String {
    let locale = init_script_locale();
    format!(
        "window.__GPM_LOCALE__ = {};",
        serde_json::to_string(&locale).expect("locale always serializes to a JS string literal")
    )
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
        // A missing file (fresh install) is normal — fall back to defaults
        // silently. A present-but-unreadable or corrupt file is a real problem:
        // it would silently revert secure_screen / locale / autosync / lock mode
        // / clear timers to defaults. Warn so that revert leaves a trace instead
        // of vanishing (the file is plaintext, so the warn carries no secret).
        let cache = match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str::<AppConfig>(&s).unwrap_or_else(|e| {
                log::warn!("app-config: corrupt app.json, using defaults: {e}");
                AppConfig::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => AppConfig::default(),
            Err(e) => {
                log::warn!("app-config: app.json unreadable, using defaults: {e}");
                AppConfig::default()
            }
        };
        Self {
            path,
            cache: Mutex::new(cache),
        }
    }

    /// Snapshot the cached config.
    pub(crate) fn get(&self) -> AppConfig {
        self.cache.lock().expect("app config lock poisoned").clone()
    }

    /// Resolve the locale the app should render in: an explicit, supported
    /// override when one is set, otherwise the system locale (normalized to a
    /// supported code). Always returns a value in [`SUPPORTED_LOCALES`]. A
    /// stale/unsupported on-disk override (including `Some("")` from a
    /// hand-edited file) degrades to system-locale resolution rather than
    /// poisoning the result — the frontend therefore reads this, not the raw
    /// `locale` field, so an unsupported value never reaches the `WebView`.
    pub(crate) fn resolved_locale(&self) -> String {
        let cfg = self.get();
        match cfg.locale.as_deref() {
            Some(explicit) if is_supported_locale(explicit) => explicit.to_string(),
            _ => normalize_system_locale(sys_locale::get_locale().as_deref()),
        }
    }

    /// Effective log level as a `log::LevelFilter`: the persisted value if set and
    /// supported, else `Info`. An unsupported on-disk value (hand-edited or from a
    /// newer build) degrades to `Info` rather than poisoning logging — mirroring
    /// `resolved_locale`'s resilience.
    #[must_use]
    pub(crate) fn effective_log_level(&self) -> log::LevelFilter {
        match self.get().log_level.as_deref() {
            Some("error") => log::LevelFilter::Error,
            Some("warn") => log::LevelFilter::Warn,
            Some("debug") => log::LevelFilter::Debug,
            // "info" and None/unsupported both resolve to Info — folding "info"
            // into the wildcard avoids clippy::match_same_arms.
            _ => log::LevelFilter::Info,
        }
    }

    /// Persist `cfg` atomically (temp + rename, mirroring
    /// `rustpass::config::save_atomic`) and update the cache.
    ///
    /// The `Mutex` is held only for the final cache swap — never across the
    /// `tokio::fs` `.await` points (the write/rename complete before the guard
    /// is taken), so there is no await-held-lock deadlock risk.
    pub(crate) async fn save(&self, cfg: &AppConfig) -> Result<(), Error> {
        let json = serde_json::to_string_pretty(cfg)?;
        let tmp = self.path.with_extension("tmp");
        tokio::fs::write(&tmp, json).await?;
        tokio::fs::rename(&tmp, &self.path).await?;
        *self.cache.lock().expect("app config lock poisoned") = cfg.clone();
        Ok(())
    }

    /// Get → mutate → save → return the updated config. Shared shape for the
    /// app-scoped setters (atomic write + cache swap under the mutex, never
    /// holding the mutex across an `.await`).
    async fn update<F: FnOnce(&mut AppConfig)>(&self, f: F) -> Result<AppConfig, Error> {
        let mut cfg = self.get();
        f(&mut cfg);
        self.save(&cfg).await?;
        Ok(cfg)
    }

    /// Set the auto-lock mode. `Idle(n)` is clamped to the allowed range first.
    pub(crate) async fn set_lock_mode(&self, mode: LockMode) -> Result<AppConfig, Error> {
        self.update(|cfg| cfg.lock_mode = clamp_lock_mode(mode))
            .await
    }

    /// Set the password-view auto-clear override (`None` ⇒ default, `Some(0)` ⇒
    /// never, else clamped to the allowed range).
    pub(crate) async fn set_view_clear_secs(&self, secs: Option<u64>) -> Result<AppConfig, Error> {
        self.update(|cfg| cfg.view_clear_secs = normalize_clear_secs(secs))
            .await
    }

    /// Set the clipboard auto-clear override (same rule as view-clear).
    pub(crate) async fn set_clipboard_clear_secs(
        &self,
        secs: Option<u64>,
    ) -> Result<AppConfig, Error> {
        self.update(|cfg| cfg.clipboard_clear_secs = normalize_clear_secs(secs))
            .await
    }

    /// Set the per-save autosync flag.
    pub(crate) async fn set_autosync(&self, enabled: bool) -> Result<AppConfig, Error> {
        self.update(|cfg| cfg.autosync = enabled).await
    }

    /// Set the persisted app-launch biometric-gate intent flag (write-only
    /// mirror of the Keystore-probed runtime state).
    pub(crate) async fn set_biometric_app_lock(&self, enabled: bool) -> Result<AppConfig, Error> {
        self.update(|cfg| cfg.biometric_app_lock = enabled).await
    }

    /// Set the persisted log level (`None` ⇒ default Info). `Some` must be one of
    /// [`LOG_LEVELS`]; a bad value returns `ConfigError`. The caller applies the
    /// runtime effect (`log::set_max_level`) so this stays a pure persistence step.
    pub(crate) async fn set_log_level(&self, level: Option<String>) -> Result<AppConfig, Error> {
        validate_log_level(level.as_deref())?;
        self.update(|cfg| cfg.log_level = level).await
    }

    /// Set the persisted color-scheme override (`None` ⇒ track system). `Some`
    /// must be one of [`SUPPORTED_THEME_MODES`]; a bad value returns
    /// `ConfigError`. The frontend applies the runtime effect (the `data-theme`
    /// attribute) on receipt, so this stays a pure persistence step mirroring
    /// `set_locale`/`set_log_level`.
    pub(crate) async fn set_theme_mode(&self, mode: Option<String>) -> Result<AppConfig, Error> {
        validate_theme_mode(mode.as_deref())?;
        self.update(|cfg| cfg.theme_mode = mode).await
    }

    /// Set the persisted three-state screen-capture mode. Rejects
    /// [`SecureScreenMode::Unknown`] (a deserialization sink, not a settable
    /// value). The frontend re-applies the route's secure state on receipt, so
    /// this stays a pure persistence step mirroring `set_theme_mode`.
    pub(crate) async fn set_secure_screen_mode(
        &self,
        mode: SecureScreenMode,
    ) -> Result<AppConfig, Error> {
        if mode == SecureScreenMode::Unknown {
            return Err(Error::new(
                ErrorCode::ConfigError,
                "Unknown is not a settable screen-capture mode",
            ));
        }
        self.update(|cfg| cfg.secure_screen_mode = Some(mode)).await
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

/// Set the three-state screen-capture protection mode and persist it. Returns
/// the updated config; the frontend re-applies the current route's secure
/// state on receipt. [`SecureScreenMode::Unknown`] is rejected — it is a
/// deserialization sink, not a value the UI may set.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn set_secure_screen_mode(
    state: State<'_, AppState>,
    mode: SecureScreenMode,
) -> Result<AppConfig, Error> {
    state.app_config.set_secure_screen_mode(mode).await
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

/// Set the color-scheme preference and persist it. `mode: null` clears the
/// override (track system); `"light"` / `"dark"` pin it. Returns the updated
/// config. The frontend re-applies the theme (the `data-theme` attribute) on
/// receipt.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn set_theme_mode(
    state: State<'_, AppState>,
    mode: Option<String>,
) -> Result<AppConfig, Error> {
    state.app_config.set_theme_mode(mode).await
}

/// The authoritative locale the app should render in. The frontend uses this at
/// boot to reconcile against the best-effort value baked into the `WebView` init
/// script (which can only carry the system locale, not a pinned preference).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn resolved_locale(state: State<'_, AppState>) -> String {
    state.app_config.resolved_locale()
}

/// The effective diagnostics log level (persisted value or `"info"` default).
/// Read by the viewer's level selector.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn get_log_level(state: State<'_, AppState>) -> String {
    match state.app_config.get().log_level.as_deref() {
        Some(lvl) if LOG_LEVELS.contains(&lvl) => lvl.to_string(),
        _ => "info".to_string(),
    }
}

/// Persist the log level and apply it at runtime immediately via
/// `log::set_max_level` (no restart — the `log` macros short-circuit at
/// `max_level`). `null` clears the override (back to the Info default).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn set_log_level(
    state: State<'_, AppState>,
    level: Option<String>,
) -> Result<(), Error> {
    state.app_config.set_log_level(level).await?;
    log::set_max_level(state.app_config.effective_log_level());
    Ok(())
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
                ..Default::default()
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
                ..Default::default()
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
                ..Default::default()
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
                ..Default::default()
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

    #[test]
    fn default_theme_mode_is_none() {
        assert!(AppConfig::default().theme_mode.is_none());
    }

    #[tokio::test]
    async fn theme_mode_roundtrips_through_save() {
        let dir = tempdir().expect("tempdir");
        let store = store_at(dir.path());
        store
            .save(&AppConfig {
                secure_screen: true,
                theme_mode: Some("dark".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let reloaded = store_at(dir.path()).get();
        assert_eq!(reloaded.theme_mode.as_deref(), Some("dark"));
    }

    #[tokio::test]
    async fn theme_mode_omitted_on_disk_when_none() {
        // skip_serializing_if keeps theme_mode out of app.json when None, so
        // existing files stay byte-identical and carry no null.
        let dir = tempdir().expect("tempdir");
        let store = store_at(dir.path());
        store
            .save(&AppConfig {
                secure_screen: true,
                theme_mode: None,
                ..Default::default()
            })
            .await
            .unwrap();
        let on_disk = std::fs::read_to_string(dir.path().join(APP_CONFIG_FILE)).unwrap();
        assert!(
            !on_disk.contains("theme_mode"),
            "theme_mode key must be absent when None; got: {on_disk}"
        );
    }

    #[test]
    fn existing_app_json_without_theme_mode_loads() {
        // An app.json written before theme_mode existed must still parse, with
        // theme_mode defaulting to None (backward compatibility — adding the
        // optional field is non-breaking, like locale).
        let dir = tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join(APP_CONFIG_FILE),
            r#"{"secure_screen":true}"#,
        )
        .unwrap();
        let cfg = store_at(dir.path()).get();
        assert!(cfg.secure_screen);
        assert!(cfg.theme_mode.is_none());
    }

    #[test]
    fn validate_theme_mode_accepts_supported_and_none() {
        assert!(validate_theme_mode(None).is_ok());
        assert!(validate_theme_mode(Some("light")).is_ok());
        assert!(validate_theme_mode(Some("dark")).is_ok());
    }

    #[test]
    fn validate_theme_mode_rejects_unknown() {
        // "system" is intentionally NOT a stored value — the frontend sends
        // `null` for "track system", so a literal "system" is rejected.
        for bad in ["system", "auto", "DARK", "", "blue"] {
            let err = validate_theme_mode(Some(bad)).unwrap_err();
            assert_eq!(err.code, "CONFIG_ERROR", "reject {bad:?}");
            assert!(
                err.message.contains(bad),
                "message names {bad:?}: {}",
                err.message
            );
        }
    }

    #[tokio::test]
    async fn set_theme_mode_persists_validates_and_clears() {
        let dir = tempdir().expect("tempdir");
        let store = store_at(dir.path());
        store
            .set_theme_mode(Some("dark".to_string()))
            .await
            .unwrap();
        assert_eq!(store.get().theme_mode.as_deref(), Some("dark"));
        // An unsupported value is rejected and must not mutate the store.
        let err = store
            .set_theme_mode(Some("blue".to_string()))
            .await
            .unwrap_err();
        assert_eq!(err.code, "CONFIG_ERROR");
        assert_eq!(store.get().theme_mode.as_deref(), Some("dark"));
        // null clears the override (track system).
        store.set_theme_mode(None).await.unwrap();
        assert!(store.get().theme_mode.is_none());
    }

    #[test]
    fn validate_log_level_accepts_supported_and_none() {
        assert!(validate_log_level(None).is_ok());
        for lvl in LOG_LEVELS {
            assert!(validate_log_level(Some(lvl)).is_ok(), "accept {lvl}");
        }
    }

    #[test]
    fn validate_log_level_rejects_unknown() {
        for bad in ["trace", "off", "DEBUG", "", "verbose"] {
            let err = validate_log_level(Some(bad)).unwrap_err();
            assert_eq!(err.code, "CONFIG_ERROR", "reject {bad:?}");
            assert!(
                err.message.contains(bad),
                "message names {bad:?}: {}",
                err.message
            );
        }
    }

    #[test]
    fn app_config_store_new_missing_file_uses_defaults() {
        let dir = tempdir().expect("tempdir");
        let store = AppConfigStore::new(dir.path());
        assert_eq!(
            store.get().secure_screen,
            AppConfig::default().secure_screen,
            "missing app.json must fall back to the secure default"
        );
    }

    #[test]
    fn app_config_store_new_corrupt_json_uses_defaults() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join(APP_CONFIG_FILE), "{not valid json").unwrap();
        let store = AppConfigStore::new(dir.path());
        assert_eq!(
            store.get().secure_screen,
            AppConfig::default().secure_screen,
            "corrupt app.json must fall back to the secure default, not panic"
        );
    }

    #[test]
    fn app_config_store_new_valid_file_loads_value() {
        let dir = tempdir().expect("tempdir");
        // A non-default value round-trips: secure_screen=false (default is true).
        std::fs::write(
            dir.path().join(APP_CONFIG_FILE),
            serde_json::json!({ "secure_screen": false }).to_string(),
        )
        .unwrap();
        let store = AppConfigStore::new(dir.path());
        assert!(
            !store.get().secure_screen,
            "a valid file's secure_screen=false must load (not revert to default)"
        );
    }

    #[tokio::test]
    async fn log_level_roundtrips_through_save() {
        let dir = tempdir().expect("tempdir");
        let store = store_at(dir.path());
        store
            .save(&AppConfig {
                secure_screen: true,
                log_level: Some("debug".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let reloaded = store_at(dir.path()).get();
        assert_eq!(reloaded.log_level.as_deref(), Some("debug"));
    }

    #[tokio::test]
    async fn log_level_omitted_on_disk_when_none() {
        let dir = tempdir().expect("tempdir");
        let store = store_at(dir.path());
        store
            .save(&AppConfig {
                secure_screen: true,
                log_level: None,
                ..Default::default()
            })
            .await
            .unwrap();
        let on_disk = std::fs::read_to_string(dir.path().join(APP_CONFIG_FILE)).unwrap();
        assert!(
            !on_disk.contains("log_level"),
            "log_level key must be absent when None; got: {on_disk}"
        );
    }

    #[tokio::test]
    async fn effective_log_level_degrades_unsupported_to_info() {
        let dir = tempdir().expect("tempdir");
        let store = store_at(dir.path());
        // A raw "trace" (rejected by set_log_level, but a hand-edited file or
        // newer build could carry it) must degrade to Info, not panic or poison.
        store
            .save(&AppConfig {
                secure_screen: true,
                log_level: Some("trace".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(store.effective_log_level(), log::LevelFilter::Info);
        // A supported value resolves directly.
        store
            .save(&AppConfig {
                secure_screen: true,
                log_level: Some("debug".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(store.effective_log_level(), log::LevelFilter::Debug);
        // None → Info: a brand-new dir (no app.json) loads the default, which
        // has no log_level and degrades to Info. Reusing `dir` would re-read
        // the "debug" persisted above and wrongly resolve to Debug.
        let fresh_dir = tempdir().expect("tempdir");
        let fresh = store_at(fresh_dir.path());
        assert_eq!(fresh.effective_log_level(), log::LevelFilter::Info);
    }

    #[test]
    fn normalize_system_locale_maps_variants() {
        assert_eq!(normalize_system_locale(None), "en");
        assert_eq!(normalize_system_locale(Some("en")), "en");
        assert_eq!(normalize_system_locale(Some("en-US")), "en");
        assert_eq!(normalize_system_locale(Some("zh")), "zh-CN");
        assert_eq!(normalize_system_locale(Some("zh-CN")), "zh-CN");
        assert_eq!(normalize_system_locale(Some("zh-Hans-CN")), "zh-CN");
        assert_eq!(normalize_system_locale(Some("zh-TW")), "zh-CN");
        // An unsupported system locale falls back to the default.
        assert_eq!(normalize_system_locale(Some("fr-FR")), "en");
    }

    #[tokio::test]
    async fn resolved_locale_uses_explicit_override() {
        let dir = tempdir().expect("tempdir");
        let store = store_at(dir.path());
        store
            .save(&AppConfig {
                secure_screen: true,
                locale: Some("zh-CN".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(store.resolved_locale(), "zh-CN");
    }

    #[tokio::test]
    async fn resolved_locale_ignores_unsupported_disk_value() {
        // A hand-edited file (or a future migration) could write an unsupported
        // code or empty string. The resolver must not surface it — it degrades
        // to a supported locale rather than handing the raw value to the UI.
        let dir = tempdir().expect("tempdir");
        let store = store_at(dir.path());
        store
            .save(&AppConfig {
                secure_screen: true,
                locale: Some("fr".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let resolved = store.resolved_locale();
        assert!(
            is_supported_locale(&resolved),
            "unsupported override must resolve to a supported locale, got {resolved}"
        );
    }

    #[test]
    fn resolved_locale_with_none_returns_supported() {
        let dir = tempdir().expect("tempdir");
        let resolved = store_at(dir.path()).resolved_locale();
        assert!(
            is_supported_locale(&resolved),
            "resolved locale must be supported, got {resolved}"
        );
    }

    #[test]
    fn init_script_locale_returns_supported() {
        // The init script runs before app.json is readable, so it carries the
        // system-locale resolution — always a supported code.
        let resolved = init_script_locale();
        assert!(
            is_supported_locale(&resolved),
            "init script locale must be supported, got {resolved}"
        );
    }

    #[test]
    fn default_secure_screen_mode_is_none() {
        assert!(AppConfig::default().secure_screen_mode.is_none());
    }

    /// `#[serde(other)]` sinks a value written by a newer build to `Unknown`
    /// instead of failing `AppConfig` deserialization (which would wipe the
    /// whole config). The frontend resolves `Unknown` to the sensitive default.
    #[test]
    fn secure_screen_mode_unknown_sinks_via_serde_other() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join(APP_CONFIG_FILE),
            r#"{"secure_screen_mode":"some-future-mode"}"#,
        )
        .unwrap();
        let cfg = store_at(dir.path()).get();
        assert_eq!(cfg.secure_screen_mode, Some(SecureScreenMode::Unknown));
    }

    #[tokio::test]
    async fn secure_screen_mode_roundtrips_through_save() {
        let dir = tempdir().expect("tempdir");
        let store = store_at(dir.path());
        for mode in [
            SecureScreenMode::Off,
            SecureScreenMode::Sensitive,
            SecureScreenMode::Always,
        ] {
            store
                .set_secure_screen_mode(mode)
                .await
                .expect("set succeeds");
            assert_eq!(
                store_at(dir.path()).get().secure_screen_mode,
                Some(mode),
                "{mode:?} round-trips",
            );
        }
    }

    #[tokio::test]
    async fn secure_screen_mode_omitted_on_disk_when_none() {
        // skip_serializing_if keeps the field out of app.json while None, so a
        // default config stays byte-identical.
        let dir = tempdir().expect("tempdir");
        let store = store_at(dir.path());
        store
            .save(&AppConfig {
                secure_screen_mode: None,
                ..Default::default()
            })
            .await
            .unwrap();
        let on_disk = std::fs::read_to_string(dir.path().join(APP_CONFIG_FILE)).unwrap();
        assert!(
            !on_disk.contains("secure_screen_mode"),
            "secure_screen_mode must be absent when None; got: {on_disk}",
        );
    }

    #[tokio::test]
    async fn set_secure_screen_mode_persists_and_rejects_unknown() {
        let dir = tempdir().expect("tempdir");
        let store = store_at(dir.path());
        store
            .set_secure_screen_mode(SecureScreenMode::Always)
            .await
            .unwrap();
        assert_eq!(
            store.get().secure_screen_mode,
            Some(SecureScreenMode::Always)
        );
        // Unknown is a deserialization sink, not a settable value.
        let err = store
            .set_secure_screen_mode(SecureScreenMode::Unknown)
            .await
            .unwrap_err();
        assert_eq!(err.code, "CONFIG_ERROR");
        // The rejected value did not mutate the store.
        assert_eq!(
            store.get().secure_screen_mode,
            Some(SecureScreenMode::Always)
        );
    }

    #[test]
    fn serde_missing_key_schema_default_stays_at_one() {
        // The serde missing-key default stays at 1: a pre-split app.json that
        // omits the key must still run the registry (otherwise it would skip
        // straight to the target and silently lose the scope split + the
        // bool→mode conversion). A brand-new config uses AppConfig::default,
        // tested below.
        assert_eq!(default_schema_version(), 1);
    }

    #[test]
    fn default_config_starts_at_current_schema_target() {
        // A brand-new install skips the legacy no-op migrations by starting at
        // the registry's target. (Existing files keep their own schema_version;
        // only a missing key falls back to the serde default of 1.)
        assert_eq!(
            AppConfig::default().schema_version,
            crate::migrations::APP_CONFIG_SCHEMA_VERSION,
        );
    }
}
