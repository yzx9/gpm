// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! App-shell configuration that must persist before any repo is set up, and
//! survive a repository re-setup.
//!
//! # A single sealed app config (R074)
//!
//! All app preferences — display (`locale`, `theme_mode`, `verbose_until`,
//! `background_sync`, `schema_version`) **and** behavior (`lock_mode`, the
//! view/clipboard clear timers, `autosync`, `biometric_app_lock`,
//! `secure_screen_mode`, `gate_idle`) — live in a single sealed `app.json`,
//! AEAD-sealed at rest on Android (passthrough-plaintext on desktop). The config
//! tier holds **zero plaintext**.
//!
//! This collapses the former two-file split (`pref.json` plaintext display +
//! sealed `app.json` behavior). The split existed only so the display prefs
//! could render before the at-rest key was available; R064 made the master key
//! **auth-free** and R074 loads it at app startup (always, including under App
//! Lock — see decision D), so the sealed config is readable at first paint and
//! the split's premise is gone. The auth-free master key is **not** what App
//! Lock protects (App Lock gates the vault key / identity), so loading it at
//! `.setup()` is safe — see [`docs/adr/A003-configuration-storage-tiering.md`].
//!
//! Internally the cache is a single [`AppConfig`] (one sealed file backs one
//! cache); [`PrefConfig`] (display) and [`BehaviorConfig`] (behavior) survive as
//! projection types — the legacy two-file on-disk shapes `m0005`/`m0006` write
//! for schema-<8 upgraders (collapsed back into one by `m0008` at schema 8) and
//! the halves surfaced by [`AppConfigStore::get_pref`] /
//! [`AppConfigStore::get_behavior`].
//!
//! `m0005` owns the historical split: it reads the legacy plaintext single-file
//! `app.json` as the schema-4 snapshot (`AppConfigV4`, defined in `m0004`),
//! writes the display half to `pref.json`, then seals the behavior half via the
//! Store. `m0008` is its inverse — it merges the two back into the sealed
//! `app.json` and deletes `pref.json`. (The `WebView`'s `localStorage` is
//! explicitly not a tier — it may be cleared by the system, so it is never
//! authoritative for settings.)
//!
//! # Versioned snapshots (V1–V4)
//!
//! Each pre-split migration (`m0002`–`m0004`) reads its own source-version
//! snapshot type raw off disk and writes its target-version snapshot raw — see
//! [`AppConfigStore::read_app_json_as`] / [`AppConfigStore::write_app_json_raw`].
//! The deprecated `secure_screen: bool` lives only in V1/V2 (consumed by
//! `m0003`); the deprecated `log_level` lives only in V1/V2/V3 (consumed by
//! `m0004`). Neither reaches the runtime types ([`PrefConfig`]/[`BehaviorConfig`]).
//!
//! The sealed `app.json` intentionally survives `reset_config` (which wipes the
//! repo dir, `identity`, `repo.json`, and the `app_id_pass` slot): it holds
//! device-level preferences, not repo data, so re-setting up the repo does not
//! reset the user's language, timers, autosync, or app-lock choice.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use rustpass::config::save_atomic;
use rustpass::{Error, ErrorCode, LockMode, Store, clamp_lock_mode, normalize_clear_secs};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tokio::fs;

use crate::AppState;
use crate::verbose::{arm_verbose_timer, disarm_verbose_timer};

/// File name of the plaintext display-prefs file (post-split).
const PREF_FILE: &str = "pref.json";
/// Sync-attention marker — a dedicated file (NOT a `pref.json` field) so the
/// headless Worker writes it atomically with no read-modify-write that could
/// race a foreground pref write. Set when a sync hits a
/// divergence / authenticity-block needing the user's review.
const SYNC_ATTENTION_FILE: &str = ".sync_attention";

/// File name of the legacy single-shape app config (pre-split) AND the sealed
/// behavior slot (post-split). The `m0005` migration repurposes it from
/// plaintext-legacy to sealed-behavior.
const APP_CONFIG_FILE: &str = "app.json";

/// Locales the app ships translations for. An explicit preference must be one
/// of these; anything else degrades to the system-locale resolution.
const SUPPORTED_LOCALES: [&str; 2] = ["en", "zh-CN"];

/// The locale used when no preference is set and the system locale is neither
/// English nor Chinese — keeps an unsupported system from rendering blank keys.
const DEFAULT_LOCALE: &str = "en";

/// How long a verbose session stays on, in seconds. Verbose lifts the runtime
/// log gate to Debug for a bounded window so it cannot be left on indefinitely;
/// the deadline persists so the window survives a restart (a relaunch made with
/// verbose on runs the whole session — including startup — at Debug). See RFC
/// 0055; ~10 min fits the rotation budget and the "capture one repro" use case.
pub(crate) const VERBOSE_WINDOW_SECS: u64 = 10 * 60;

/// Current time as Unix seconds. A pre-epoch clock skew (impossible in practice)
/// degrades to `0` ⇒ any deadline reads as expired ⇒ Info, which is safe.
pub(crate) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Localized text for the verbose-revert OS notification. Staged when verbose is
/// enabled (the frontend passes the already-localized title/body) and consumed —
/// posted by the deadline timer — when the window elapses. Held in memory only
/// (UI text, not config). If absent at fire time (a relaunch re-armed the timer
/// with no staged text), the revert still happens; the level + on-disk deadline
/// are the source of truth, the notice is simply skipped.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VerboseNotifyText {
    pub(crate) title: String,
    pub(crate) body: String,
}

/// Three-state screen-capture protection mode. Serialized kebab-case as
/// `"off"` / `"sensitive"` / `"always"`. [`SecureScreenMode::Unknown`] is a
/// forward-compatibility sink (`#[serde(other)]`): a value written by a newer
/// build deserializes to `Unknown` instead of failing the surrounding config
/// parse (which would wipe the whole config back to defaults). The frontend
/// treats `None` and `Unknown` as the sensitive default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SecureScreenMode {
    Off,
    Sensitive,
    Always,
    #[serde(other)]
    Unknown,
}

/// App-launch-biometric-gate in-app idle timeout (sealed behavior). `Off` =
/// never idle-lock the gate; `After(n)` = lock after `n` seconds
/// foregrounded-but-idle. Both variants persist — a user who turns it off keeps
/// that choice across reloads — so this is an enum, not `Option<u64>` (with a
/// unified non-`None` default, `None`-means-off could not persist through
/// `skip_serializing_if`). Mirrors `LockMode`'s shape (a lock-timeout mode).
/// Serialized externally-tagged kebab-case: `"off"` / `{"after": secs}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum GateIdle {
    Off,
    After(u64),
}

/// The gate idle-timeout default for new installs (5 min). Deliberately coarser
/// than the identity's 30s floor: a whole-store lock (master-key wipe → full
/// biometric + re-seal) is heavier than an identity re-lock, so the gate
/// shouldn't fire that often. Existing configs are migrated to `Off` by `m0006`
/// (a deliberate, discoverable default choice — not a struct-vs-serde split).
pub(crate) const GATE_IDLE_DEFAULT_SECS: u64 = 300;

/// The gate idle-timeout preset floor/ceiling (5 / 30 min). Sub-5-min values
/// aren't offered — a whole-store lock that often is too disruptive.
pub(crate) const GATE_IDLE_SECS_MIN: u64 = 300;
pub(crate) const GATE_IDLE_SECS_MAX: u64 = 1800;

impl Default for GateIdle {
    /// `After(GATE_IDLE_DEFAULT_SECS)` — the single, unified default used by
    /// BOTH `AppConfig::default()` / `BehaviorConfig::default()` and the serde
    /// missing-key default. No divergence between the two.
    fn default() -> Self {
        Self::After(GATE_IDLE_DEFAULT_SECS)
    }
}

impl GateIdle {
    /// True at the unified default, so `skip_serializing_if` omits it and a
    /// fresh, uncustomized config stays byte-identical.
    fn is_default(&self) -> bool {
        matches!(self, Self::After(GATE_IDLE_DEFAULT_SECS))
    }
}

/// Clamp `After(n)` to the gate's preset range; `Off` passes through. Mirrors
/// `rustpass::clamp_lock_mode`.
pub(crate) fn clamp_gate_idle(mode: GateIdle) -> GateIdle {
    match mode {
        GateIdle::After(secs) => {
            GateIdle::After(secs.clamp(GATE_IDLE_SECS_MIN, GATE_IDLE_SECS_MAX))
        }
        GateIdle::Off => GateIdle::Off,
    }
}

/// App-level (non-repo) preferences — the **single sealed on-disk shape** at
/// schema 8 (the merged display + behavior config). [`AppConfigStore::get`]
/// clones it straight from the one in-memory cache; [`AppConfigStore::save_merged`]
/// persists it to the sealed `app.json`. Display vs behavior is a historical
/// split (the pre-R074 two-file layout): [`PrefConfig`] / [`BehaviorConfig`] are
/// its projection types, and the legacy single-file shape (schema 1–4) is
/// preserved as [`LegacyAppConfig`] for the pre-split lift.
///
/// Field docs describe semantics, not storage — at schema 8 every field lives in
/// the one merged file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AppConfig {
    /// Three-state screen-capture protection. `None` (the
    /// default) ⇒ `Sensitive` (the frontend resolves `None`/`Unknown` to
    /// `Sensitive`): sensitive routes + nav transitions + the unlock overlay
    /// block capture, the entry list / history stay capturable. `Off` ⇒ no
    /// screen is ever secured (the user explicitly allowed capture, including
    /// the unlock overlay). `Always` ⇒ every screen is secured at all times.
    /// `skip_serializing_if` keeps the field out of `app.json` while `None`, so
    /// a default config stays byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) secure_screen_mode: Option<SecureScreenMode>,
    /// Display-language override. `None` (the default) means "track
    /// the system language" — the backend resolves the system locale at boot.
    /// `Some("en")` / `Some("zh-CN")` pins the locale explicitly.
    /// `skip_serializing_if` keeps existing files (which predate this field)
    /// byte-identical on round-trip, so adding the field is non-breaking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) locale: Option<String>,
    /// Color-scheme (light/dark) override. `None` (the default)
    /// means "track the system preference" — the frontend's
    /// `prefers-color-scheme` CSS media query governs, zero-JS and zero-flash.
    /// `Some("light")` / `Some("dark")` pins it via a `<html data-theme>`
    /// attribute the frontend sets after reading this. `skip_serializing_if`
    /// keeps existing files byte-identical on round-trip, so adding the field is
    /// non-breaking (mirrors `locale`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) theme_mode: Option<String>,
    /// How the app auto-locks the identity cache. Skipped
    /// from serialization when default (`Immediate`), so an uncustomized config
    /// is byte-identical to one written before this field moved here.
    #[serde(default, skip_serializing_if = "LockMode::is_default")]
    pub(crate) lock_mode: LockMode,
    /// Seconds a revealed password stays in the DOM before auto-clear.
    /// `None` ⇒ [`DEFAULT_VIEW_CLEAR_SECS`]; `Some(0)` ⇒ never
    /// auto-clear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) view_clear_secs: Option<u64>,
    /// Seconds the clipboard holds a copied password before auto-clear.
    /// `None` ⇒ [`DEFAULT_CLIPBOARD_CLEAR_SECS`]; `Some(0)` ⇒ never
    /// auto-clear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) clipboard_clear_secs: Option<u64>,
    /// Whether each save wraps in a pull→write→push (gopass-style per-command
    /// sync). Default `true`; omitted from serialization
    /// while `true`.
    #[serde(
        default = "default_autosync_true",
        skip_serializing_if = "is_autosync_default"
    )]
    pub(crate) autosync: bool,
    /// Persisted intent for the app-launch biometric gate.
    /// **Write-only** — the Settings toggle and the runtime gate read the
    /// Keystore probe via `get_app_lock_state`, not this flag; it exists only
    /// as a persisted record mirroring the old `RepoConfig` field. Skipped when
    /// `false`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) biometric_app_lock: bool,
    /// App-launch-gate in-app idle timeout. Defaults to
    /// `After(300)` (5 min) for new installs; `m0006` sets existing configs to
    /// `Off`. `skip_serializing_if` omits the default so a fresh config stays
    /// byte-identical.
    #[serde(default, skip_serializing_if = "GateIdle::is_default")]
    pub(crate) gate_idle: GateIdle,
    /// Persisted-schema version for one-shot migrations. `1` is the
    /// pre-split shape; the `migrations` registry bumps it as each step runs
    /// (target: `migrations::APP_CONFIG_SCHEMA_VERSION`).
    #[serde(default = "default_schema_version")]
    pub(crate) schema_version: u32,
    /// Verbose-logging deadline as Unix seconds, or `None` (the Info default).
    /// While set and not yet past, the runtime log gate is Debug; once past (or
    /// `None`) it is Info. A launch made with verbose on records startup at
    /// Debug. See RFC 0055. Omitted while `None` so a default config stays
    /// byte-identical on round-trip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) verbose_until: Option<u64>,
    /// Periodic background-sync cadence. `Off` (default) omitted.
    #[serde(default, skip_serializing_if = "BackgroundSyncCadence::is_off")]
    pub(crate) background_sync: BackgroundSyncCadence,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            secure_screen_mode: None,
            locale: None,
            theme_mode: None,
            lock_mode: LockMode::default(),
            view_clear_secs: None,
            clipboard_clear_secs: None,
            autosync: default_autosync_true(),
            biometric_app_lock: false,
            gate_idle: GateIdle::default(),
            // A brand-new config starts at the current target so it skips the
            // legacy no-op migrations. (The serde missing-key default below
            // stays at 1 so a pre-split app.json still runs the registry.)
            schema_version: crate::migrations::APP_CONFIG_SCHEMA_VERSION,
            verbose_until: None,
            background_sync: BackgroundSyncCadence::default(),
        }
    }
}

impl AppConfig {
    /// Compose the merged [`AppConfig`] from its display (`pref`) and behavior
    /// halves. Used by [`AppConfigStore::new`] / [`AppConfigStore::reload`] /
    /// [`AppConfigStore::reload_behavior`] to (re)build the single cache, and by
    /// `m0008` to merge before sealing. Mirrors the field mapping in
    /// [`PrefConfig::from_app`] / [`BehaviorConfig::from_app`] (the inverses).
    pub(crate) fn from_halves(pref: &PrefConfig, behavior: &BehaviorConfig) -> Self {
        Self {
            secure_screen_mode: behavior.secure_screen_mode,
            locale: pref.locale.clone(),
            theme_mode: pref.theme_mode.clone(),
            lock_mode: behavior.lock_mode,
            view_clear_secs: behavior.view_clear_secs,
            clipboard_clear_secs: behavior.clipboard_clear_secs,
            autosync: behavior.autosync,
            biometric_app_lock: behavior.biometric_app_lock,
            gate_idle: behavior.gate_idle,
            schema_version: pref.schema_version,
            verbose_until: pref.verbose_until,
            background_sync: pref.background_sync,
        }
    }
}

/// Periodic background-sync cadence. `Off` (the default) is opt-in — no
/// periodic background sync; the foreground sync (cold-start/resume/unlock)
/// still runs. `1h`..`3d` enqueues an Android `WorkManager` periodic **pull**.
/// Linked to `AutoSync`: background sync runs only when `AutoSync` is on AND
/// cadence is not `Off`. Readable pre-unlock via the auth-free key, so the
/// headless worker can read it without the vault key.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum BackgroundSyncCadence {
    #[default]
    #[serde(rename = "off")]
    Off,
    #[serde(rename = "1h")]
    Hours1,
    #[serde(rename = "6h")]
    Hours6,
    #[serde(rename = "12h")]
    Hours12,
    #[serde(rename = "1d")]
    Days1,
    #[serde(rename = "3d")]
    Days3,
}

impl BackgroundSyncCadence {
    /// `Off` is the default; omit it from serialization to keep `pref.json` tidy.
    #[allow(clippy::trivially_copy_pass_by_ref)] // serde `skip_serializing_if` passes `&T`.
    pub(crate) fn is_off(&self) -> bool {
        *self == Self::Off
    }

    /// The scheduling interval in hours, or `None` for `Off`. (Used in the
    /// Android-only scheduling path; `dead_code` on desktop targets.)
    #[must_use]
    #[allow(dead_code)]
    pub(crate) fn hours(self) -> Option<u64> {
        match self {
            Self::Off => None,
            Self::Hours1 => Some(1),
            Self::Hours6 => Some(6),
            Self::Hours12 => Some(12),
            Self::Days1 => Some(24),
            Self::Days3 => Some(72),
        }
    }
}

/// Display preferences — the projection of [`AppConfig`]'s display half
/// (`locale`/`theme_mode`/`verbose_until`/`schema_version`/`background_sync`),
/// surfaced by [`AppConfigStore::get_pref`] and the diagnostics locked
/// projection. It doubles as the on-disk `pref.json` shape `m0005`–`m0007`
/// write for schema-<8 upgraders (`m0008` collapses it back into the sealed
/// merged `app.json` at schema 8).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PrefConfig {
    /// Display-language override. `None` (the default) means "track the system
    /// language". `skip_serializing_if` keeps existing files byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) locale: Option<String>,
    /// Color-scheme override. `None` (the default) means "track the system
    /// preference". `skip_serializing_if` keeps existing files byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) theme_mode: Option<String>,
    /// Verbose-logging deadline as Unix seconds, or `None` (the Info default).
    /// Drives the runtime log gate via [`AppConfigStore::effective_log_filter`].
    /// `skip_serializing_if` keeps existing files byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) verbose_until: Option<u64>,
    /// Persisted-schema version for one-shot migrations. `1` is the pre-split
    /// shape; the `migrations` registry bumps it as each step runs. The serde
    /// missing-key default stays at `1` so a pre-split `app.json` (lifted into
    /// a `PrefConfig` on first read) still runs the registry; a brand-new
    /// install is built via [`PrefConfig::default`], which starts at the
    /// registry's target instead (skipping the legacy no-op steps) — the two
    /// differ on purpose.
    #[serde(default = "default_schema_version")]
    pub(crate) schema_version: u32,
    /// Periodic background-sync cadence (see [`BackgroundSyncCadence`]). `Off`
    /// (the default) is omitted from serialization.
    #[serde(default, skip_serializing_if = "BackgroundSyncCadence::is_off")]
    pub(crate) background_sync: BackgroundSyncCadence,
}

impl Default for PrefConfig {
    fn default() -> Self {
        Self {
            locale: None,
            theme_mode: None,
            verbose_until: None,
            // A brand-new config starts at the current target so it skips the
            // legacy no-op migrations. (The serde missing-key default below
            // stays at 1 so a pre-split app.json still runs the registry.)
            schema_version: crate::migrations::APP_CONFIG_SCHEMA_VERSION,
            background_sync: BackgroundSyncCadence::default(),
        }
    }
}

impl PrefConfig {
    /// Lift the display half of a [`LegacyAppConfig`] (legacy single-file shape)
    /// into a [`PrefConfig`]. Used by [`AppConfigStore::new`] for the legacy
    /// lift and by the engine's end-of-chain [`AppConfigStore::reload`]. The
    /// deprecated `secure_screen`/`log_level` fields live only in the version
    /// snapshots (V1–V3) — neither survives into the runtime types.
    pub(crate) fn from_legacy(cfg: &LegacyAppConfig) -> Self {
        Self {
            locale: cfg.locale.clone(),
            theme_mode: cfg.theme_mode.clone(),
            verbose_until: cfg.verbose_until,
            schema_version: cfg.schema_version,
            background_sync: BackgroundSyncCadence::default(),
        }
    }

    /// Project the display half out of a merged [`AppConfig`] — the inverse of
    /// [`AppConfig::from_halves`]. Used by [`AppConfigStore::get_pref`] (the
    /// projection) and [`AppConfigStore::reload_behavior`] (to preserve the
    /// display half when loading the behavior slot).
    pub(crate) fn from_app(cfg: &AppConfig) -> Self {
        Self {
            locale: cfg.locale.clone(),
            theme_mode: cfg.theme_mode.clone(),
            verbose_until: cfg.verbose_until,
            schema_version: cfg.schema_version,
            background_sync: cfg.background_sync,
        }
    }
}

/// Behavior preferences — the projection of [`AppConfig`]'s behavior half (the
/// confidential security choices: lock timeout, autosync, biometric,
/// screen-capture mode), surfaced by [`AppConfigStore::get_behavior`]. It
/// doubles as the on-disk sealed-`app.json` behavior-slot shape `m0005`/`m0006`
/// write for schema-<8 upgraders. On Android the slot is AEAD-sealed under the
/// auth-free master key (readable at `.setup()`); on desktop the seal is
/// passthrough plaintext. Same serde attrs as the equivalent [`AppConfig`]
/// fields so the slot's shape mirrors the legacy single-file shape
/// byte-for-byte (modulo the missing display keys).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BehaviorConfig {
    /// How the app auto-locks the identity cache. Skipped from serialization
    /// when default (`Immediate`).
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
    /// Persisted intent for the app-launch biometric gate. **Write-only** —
    /// the Settings toggle and the runtime gate read the Keystore probe, not
    /// this flag. Skipped when `false`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) biometric_app_lock: bool,
    /// App-launch-gate in-app idle timeout. Defaults to `After(300)` (5 min) for
    /// new installs; `m0006` sets existing configs to `Off`. `skip_serializing_if`
    /// omits the default so the behavior file stays byte-identical.
    #[serde(default, skip_serializing_if = "GateIdle::is_default")]
    pub(crate) gate_idle: GateIdle,
    /// Three-state screen-capture protection. `None` (the default) ⇒
    /// `Sensitive`. `skip_serializing_if` keeps the field out while `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) secure_screen_mode: Option<SecureScreenMode>,
}

impl Default for BehaviorConfig {
    fn default() -> Self {
        Self {
            lock_mode: LockMode::default(),
            view_clear_secs: None,
            clipboard_clear_secs: None,
            autosync: default_autosync_true(),
            biometric_app_lock: false,
            gate_idle: GateIdle::default(),
            secure_screen_mode: None,
        }
    }
}

impl BehaviorConfig {
    /// Lift the behavior half of a [`LegacyAppConfig`] (legacy single-file
    /// shape) into a [`BehaviorConfig`]. Used by [`AppConfigStore::new`] /
    /// [`AppConfigStore::reload`] for the legacy lift (the half-migrated state
    /// where `pref.json` exists at schema < 5 but `app.json` is still the
    /// plaintext single-file shape).
    pub(crate) fn from_legacy(cfg: &LegacyAppConfig) -> Self {
        Self {
            lock_mode: cfg.lock_mode,
            view_clear_secs: cfg.view_clear_secs,
            clipboard_clear_secs: cfg.clipboard_clear_secs,
            autosync: cfg.autosync,
            biometric_app_lock: cfg.biometric_app_lock,
            // gate_idle is a new field — a legacy config never had it, so the
            // lift defaults it (m0006 later pins existing users to Off).
            gate_idle: GateIdle::default(),
            secure_screen_mode: cfg.secure_screen_mode,
        }
    }

    /// Project the behavior half out of a merged [`AppConfig`] — the inverse of
    /// [`AppConfig::from_halves`]. Used by [`AppConfigStore::get_behavior`] (the
    /// projection) and [`AppConfigStore::reload`] (to preserve the behavior half
    /// when reloading pref.json).
    pub(crate) fn from_app(cfg: &AppConfig) -> Self {
        Self {
            lock_mode: cfg.lock_mode,
            view_clear_secs: cfg.view_clear_secs,
            clipboard_clear_secs: cfg.clipboard_clear_secs,
            autosync: cfg.autosync,
            biometric_app_lock: cfg.biometric_app_lock,
            gate_idle: cfg.gate_idle,
            secure_screen_mode: cfg.secure_screen_mode,
        }
    }
}

/// The legacy single-file `app.json` shape (schema 1–4). Deserialize-only in
/// practice — used by [`AppConfigStore::new`] (the pre-split lift) and the
/// engine's end-of-chain [`AppConfigStore::reload`] (the half-migrated behavior
/// lift) to pull display and/or behavior fields back into the cache. The
/// `Serialize` derive is only there so legacy round-trip tests can call
/// [`AppConfigStore::save_legacy_app_json`] to seed a pre-split file shape —
/// post-split writes go through [`AppConfigStore::save_pref`] / [`AppConfigStore::save_behavior`]
/// instead. Permissive on read: accepts any historical shape (V1/V2/V3/V4)
/// since the deprecated `secure_screen: bool` and `log_level` ride along as
/// ordinary fields (no `deny_unknown_fields`). Not the runtime type — post-split
/// display lives in [`PrefConfig`] and behavior in [`BehaviorConfig`].
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct LegacyAppConfig {
    /// **Deprecated** boolean master toggle (consumed by `m0003`; lives only
    /// in V1/V2). Default ON (`true`) — see [`default_secure_screen`].
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
    #[serde(default = "default_schema_version")]
    pub(crate) schema_version: u32,
    /// **Deprecated** persisted diagnostics level (consumed by `m0004`; lives
    /// only in V1/V2/V3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) log_level: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) verbose_until: Option<u64>,
}

/// Manual `Default` for [`LegacyAppConfig`] so `secure_screen` matches its serde
/// default (`true`), not the derived `bool` default (`false`). Callers fall
/// back to `LegacyAppConfig::default()` for fields they don't explicitly set
/// (e.g. `..Default::default()` in tests); a derived `false` for `secure_screen`
/// would disagree with the serde default and produce a different lift than a
/// missing key — pinning the agreement here is the safer choice.
impl Default for LegacyAppConfig {
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
            schema_version: default_schema_version(),
            log_level: None,
            verbose_until: None,
        }
    }
}

/// Serde default for the deprecated `secure_screen: bool` carried by the
/// pre-split snapshot types (V1/V2) and the legacy lift — `true` (secure by
/// default). The latest runtime types no longer have this field.
pub(crate) fn default_secure_screen() -> bool {
    true
}

/// Serde default for `autosync` — `true` (gopass-style per-save pull→write→push
/// on by default). Shared by [`AppConfig`], [`BehaviorConfig`], and the version
/// snapshots that carry the field.
pub(crate) fn default_autosync_true() -> bool {
    true
}

/// `true` (the default) so `autosync` is omitted from the file while on — a
/// user who never toggles it sees no change to the file's shape.
#[allow(clippy::trivially_copy_pass_by_ref)] // serde's skip_serializing_if needs `fn(&T)`
pub(crate) fn is_autosync_default(autosync: &bool) -> bool {
    *autosync
}

/// `false` (the default) so `biometric_app_lock` is omitted from the file when
/// off.
#[allow(clippy::trivially_copy_pass_by_ref)] // serde's skip_serializing_if needs `fn(&T)`
pub(crate) fn is_false(b: &bool) -> bool {
    !*b
}

/// Serde default for the schema-version field when the key is missing — `1`,
/// the version before the config-scope migration existed. A pre-split file that
/// omits the key must still run the registry (otherwise it would skip straight
/// to the target and silently lose the scope split + the bool→mode conversion +
/// the sealed-behavior split), so this stays at `1`. A brand-new install is
/// built via [`AppConfig::default`] / [`PrefConfig::default`], which start at
/// `APP_CONFIG_SCHEMA_VERSION` instead (skipping the legacy no-op steps) — the
/// two differ on purpose. The V2/V3/V4 snapshots define their own version-local
/// defaults.
pub(crate) fn default_schema_version() -> u32 {
    1
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
/// byte-identical-on-default invariant `locale`/`verbose_until` rely on.
const SUPPORTED_THEME_MODES: [&str; 2] = ["light", "dark"];

/// Reject an unsupported explicit theme mode. `None` (track system) is always
/// valid; `Some(mode)` must be in [`SUPPORTED_THEME_MODES`]. Mirrors
/// `validate_locale`.
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

/// Serialize `value` to a JSON literal, escaped for an HTML `<script>` context.
///
/// `</` is escaped to `<\/` so a planted value can't close the injected
/// `<script>` tag early on Android `WebViews` without `addDocumentStartJavaScript`
/// (`serde_json` doesn't escape `/`; `<\/` is JSON-valid, JS-equivalent, and
/// HTML-safe). Shared by the locale and theme init scripts.
fn init_script_json<T: Serialize + ?Sized>(value: &T) -> String {
    serde_json::to_string(value)
        .expect("init-script values serialize to a JSON literal")
        .replace("</", "<\\/")
}

/// Wrap `body` in an immediately-invoked function expression so the script's
/// locals (e.g. `var d`) stay scoped and don't leak into the page's global
/// scope. Shared by the locale and theme init scripts.
fn with_iife(body: &str) -> String {
    format!("(function(){{{body}}})();")
}

/// The JavaScript snippet that bakes the resolved locale into the `WebView`
/// **before first paint**, as both `window.__GPM_LOCALE__` and `<html lang>`.
///
/// Composed in `.setup()` from [`AppConfigStore::resolved_locale`] — the pinned
/// locale when one is set, otherwise the normalized system locale — and
/// registered per-window (on the `WebviewWindowBuilder`), because it can only be
/// composed once the sealed app config has been loaded (unreadable at Tauri
/// `Builder` time on Android). The frontend reads `__GPM_LOCALE__` synchronously at module
/// load, so the right value here means the mount frame renders the pinned
/// language with no one-frame system-locale flash.
///
/// Setting `<html lang>` too closes the parallel a11y gap: `index.html` hardcodes
/// `lang="en"`, so until the frontend sets it a screen reader on a pinned non-en
/// device reads English pronunciation. The init script runs pre-HTML-parse
/// (document start), where `document.documentElement` already exists; the
/// `if (d)` guard keeps the `lang` set a harmless no-op if it ever does not. The
/// value is always a supported code from `resolved_locale`, so no sanitization is
/// needed beyond the shared [`init_script_json`] escape.
pub(crate) fn locale_init_script(locale: &str) -> String {
    let json = init_script_json(locale);
    with_iife(&format!(
        "window.__GPM_LOCALE__ = {json};var d=document.documentElement;if(d){{d.lang={json};}}"
    ))
}

/// The JavaScript snippet that bakes the pinned color-scheme preference into
/// the `WebView` as `<html data-theme>` **before first paint**, eliminating the
/// one-frame flash a pinned Light/Dark would otherwise show (the CSS
/// `color-scheme` and app-color variables both key off `[data-theme]`).
///
/// Registered per-window (on the `WebviewWindowBuilder` inside `.setup()`),
/// alongside [`locale_init_script`], because both can only be composed once
/// the sealed app config has been loaded — which needs the running app's
/// config dir (unreadable at Tauri `Builder` time on Android).
///
/// `None` (track system) clears any stale `data-theme` — defensive, since the
/// attribute starts absent on a fresh document. `Some("light")`/`Some("dark")`
/// set it so the matching `:root[data-theme="..."]` rule (and its
/// `color-scheme`) apply from frame 0. The script runs pre-HTML-parse (document
/// start); `document.documentElement` already exists then, but the `if (d)`
/// guard keeps it a harmless no-op if it ever does not (the post-mount
/// `reconcile` then owns the value). Only `light`/`dark` are whitelisted inside
/// the JS — a garbage value (a corrupt `pref.json`) degrades to system instead
/// of poisoning frame 0, mirroring `normalize_theme_mode`.
pub(crate) fn theme_init_script(theme_mode: Option<&str>) -> String {
    let mode_json = init_script_json(&theme_mode);
    with_iife(&format!(
        "var m={mode_json},d=document.documentElement;if(d){{if(m==='light'||m==='dark'){{d.setAttribute('data-theme',m);}}else{{d.removeAttribute('data-theme');}}}}"
    ))
}

/// Read `app.json` (the pre-split single-file shape) from `config_dir` and
/// parse it as the [`LegacyAppConfig`] shape. Returns `None` if the file is
/// missing or unparseable — used by [`AppConfigStore::new`] (the legacy lift),
/// the engine's end-of-chain reload, and `m0005`'s half-migrated recovery. The
/// byte-oriented sealed behavior slot (post-split) does NOT parse as
/// [`LegacyAppConfig`] cleanly (carries only the behavior subset), so callers
/// dispatching on the file shape should check [`rustpass::seal::is_envelope`]
/// first to tell a sealed slot apart from a plaintext legacy file.
async fn load_legacy_app_json_at(path: &Path) -> Option<LegacyAppConfig> {
    let s = fs::read_to_string(path).await.ok()?;
    serde_json::from_str::<LegacyAppConfig>(&s).ok()
}

/// Minimal projection of any `app.json`/`pref.json` shape for the migration
/// engine's version gate — only `schema_version` is read. A missing key defaults
/// to 1 (the pre-split shape), matching [`default_schema_version`].
#[derive(Deserialize)]
struct SchemaVersionPeek {
    #[serde(default = "default_schema_version")]
    schema_version: u32,
}

/// Outcome of [`AppConfigStore::peek_schema_version`] — the migration engine's
/// 3-state gate (R074). `schema_version` moved from plaintext `pref.json` into
/// the sealed merged `app.json`, so the gate must unseal to read it; because the
/// auth-free master key is loaded at `.setup()` (decision D), a present `app.json`
/// is always unsealable, so there is no "deferred" state.
///
/// - `Version(v)` — `pref.json` present (old world, schema < 8) OR the sealed
///   merged `app.json` read ok (new world, schema 8). The engine runs migrations
///   whose target exceeds `v`.
/// - `Absent` — no config file at all (fresh install / post-reset). The engine
///   skips the whole chain (a missing state is not a schema to migrate).
/// - `Corrupt` — `app.json` is present but unseals/parses as garbage (real
///   tamper / lost key). The engine halts + logs so the user routes to re-setup
///   rather than silently wiping their prefs (never silently `Absent`).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PeekOutcome {
    Version(u32),
    Absent,
    Corrupt,
}

/// Persistent app-shell config, owned by [`AppState`]. Two-phase: constructed
/// without a `Store` (so the migration registry can run before the Store is
/// built if needed), then [`set_store`](Self::set_store) binds the Store so
/// sealed writes/reads can flow. At construction `pref.json` is read once via
/// `tokio::fs` (lifting the legacy `app.json` fields when `pref.json` is absent);
/// the sealed merged `app.json` is loaded post-unlock via
/// [`reload_behavior`](Self::reload_behavior). The single in-memory cache is
/// authoritative thereafter; the [`Mutex`] guard is never held across an
/// `.await`.
///
/// R074 collapsed the former two caches (`pref` + `behavior`) into one
/// `Mutex<AppConfig>` — one sealed file backs one cache. Runtime reads
/// ([`get`](Self::get)) are a single clone and writes ([`update`](Self::update))
/// swap the whole cache, so a read can never observe a half-updated state. The
/// display/behavior split survives only in the projection types
/// ([`PrefConfig`]/[`BehaviorConfig`], via [`get_pref`](Self::get_pref) /
/// [`get_behavior`](Self::get_behavior)) and the legacy on-disk shapes the
/// permanent migration registry writes.
///
/// ```text
///  pref.json (old world) ─┐
///  app.json  (sealed)   ─┴──► load ──► config: Mutex<AppConfig>
///                                        ▲               ▲
///                 get() = clone ─────────┘               │
///                 update() = clone → mutate → save → swap (held under write_mu)
///                 save_pref/save_behavior = write file → reload()
/// ```
#[derive(Debug)]
pub(crate) struct AppConfigStore {
    pref_path: PathBuf,
    app_json_path: PathBuf,
    /// The single in-memory cache: the merged display+behavior [`AppConfig`].
    /// [`get`](Self::get) clones it; [`update`](Self::update) clones→mutates→
    /// saves→swaps it under `write_mu`. [`PrefConfig`]/[`BehaviorConfig`] are
    /// projections ([`get_pref`](Self::get_pref) / [`get_behavior`](Self::get_behavior)),
    /// not separate caches.
    config: Mutex<AppConfig>,
    /// Late-bound Store ref so setter signatures stay stable (no `&Store`
    /// parameter) and so callers in `config.rs`/`applock.rs` don't change. Set
    /// once via [`set_store`](Self::set_store) right after the Store is built.
    store: OnceLock<Arc<Store>>,
    /// Serializes runtime config writes ([`update`](Self::update)'s
    /// read-modify-write of the single sealed file). Without it two concurrent
    /// setters could each clone the same cache, mutate, and the second save
    /// would overwrite the first (last-write-wins loses an update). An async
    /// mutex (not a `std::sync::Mutex`) because it is held across the
    /// sealed-write `.await`; the std `config` mutex is held only for the quick
    /// clone/swap, never across an `.await`.
    write_mu: tokio::sync::Mutex<()>,
    /// Staged text for the verbose-revert OS notification (posted by the
    /// deadline timer). Memory-only; `None` until verbose is enabled.
    revert_notify: Mutex<Option<VerboseNotifyText>>,
}

impl AppConfigStore {
    /// Load the display prefs from `config_dir/pref.json`, falling back to the
    /// legacy lift from `config_dir/app.json` when `pref.json` is absent (the
    /// pre-split case), and finally to defaults. The behavior half of the cache
    /// starts at default — sealed behavior is loaded post-unlock via
    /// [`reload_behavior`](Self::reload_behavior).
    ///
    /// Resilience: a missing file (fresh install) is normal — silent default.
    /// A present-but-unreadable or corrupt file would silently revert
    /// `locale`/`theme_mode`/`verbose_until` to defaults; warn so the revert
    /// leaves a trace (the file is plaintext, so the warn carries no secret).
    #[must_use]
    pub(crate) async fn new(config_dir: &Path) -> Self {
        let pref_path = config_dir.join(PREF_FILE);
        let app_json_path = config_dir.join(APP_CONFIG_FILE);
        // Build the single AppConfig cache. Prefer pref.json (post-split display
        // shape); fall back to the legacy lift from app.json (which carries both
        // display and behavior pre-split, so seed both halves); finally default.
        // schema_version is preserved (the registry bumps it as migrations run).
        let config = if fs::try_exists(&pref_path).await.unwrap_or(false) {
            let pref = match fs::read_to_string(&pref_path).await {
                Ok(s) => serde_json::from_str::<PrefConfig>(&s).unwrap_or_else(|e| {
                    log::warn!("app-config: corrupt pref.json, using defaults: {e}");
                    PrefConfig::default()
                }),
                Err(e) => {
                    log::warn!("app-config: pref.json unreadable, using defaults: {e}");
                    PrefConfig::default()
                }
            };
            AppConfig::from_halves(&pref, &BehaviorConfig::default())
        } else if let Some(legacy) = load_legacy_app_json_at(&app_json_path).await {
            AppConfig::from_halves(
                &PrefConfig::from_legacy(&legacy),
                &BehaviorConfig::from_legacy(&legacy),
            )
        } else {
            AppConfig::default()
        };
        Self {
            pref_path,
            app_json_path,
            config: Mutex::new(config),
            store: OnceLock::new(),
            write_mu: tokio::sync::Mutex::new(()),
            revert_notify: Mutex::new(None),
        }
    }

    /// Stage (or clear, on `None`) the localized text for the verbose-revert OS
    /// notification. Set when verbose is enabled; the deadline timer consumes it.
    pub(crate) fn set_revert_notify(&self, text: Option<VerboseNotifyText>) {
        *self
            .revert_notify
            .lock()
            .expect("revert_notify lock poisoned") = text;
    }

    /// Take the staged revert-notification text (or `None` if none was staged).
    /// The deadline timer calls this at fire time — consuming ensures a single
    /// post even if the timer somehow fires twice.
    pub(crate) fn take_revert_notify(&self) -> Option<VerboseNotifyText> {
        self.revert_notify
            .lock()
            .expect("revert_notify lock poisoned")
            .take()
    }

    /// Bind the Store ref so sealed behavior writes/reads can flow. Called once
    /// from `init_state` after the Store is constructed. Idempotent: a second
    /// call is silently dropped (the first Store wins, mirroring `OnceLock`
    /// semantics) — tests that re-construct an `AppState` over a temp dir per
    /// case never collide in practice.
    pub(crate) fn set_store(&self, store: Arc<Store>) {
        let _ = self.store.set(store);
    }

    /// Path to the legacy / sealed `app.json`. Used by `m0005` to dispatch on
    /// the file shape (missing / envelope / plaintext-legacy).
    pub(crate) fn app_json_path(&self) -> &Path {
        &self.app_json_path
    }

    /// Whether `pref.json` exists on disk — i.e. the display half has already
    /// been split off. `m0005` gates its display-half write on this so a re-entry
    /// (a `Pending` resume, or a half-migrated crash recovery) never re-derives
    /// display prefs from `app.json` and clobbers the user's locale/theme.
    /// `save_atomic` (temp + rename) guarantees the file is either absent or
    /// complete, so existence is a reliable split signal. Post-R074 (schema 8)
    /// `pref.json` is gone (collapsed into the sealed merged `app.json`), so this
    /// also discriminates the old-world (pref.json present) from the new-world
    /// (absent) read paths.
    pub(crate) async fn pref_json_exists(&self) -> bool {
        fs::try_exists(&self.pref_path).await.unwrap_or(false)
    }

    /// Path of the plaintext display-prefs file. `m0008` deletes this once it
    /// has collapsed the display prefs into the sealed merged `app.json`.
    pub(crate) fn pref_json_path(&self) -> &Path {
        &self.pref_path
    }

    /// Read `app.json` (the pre-split single-file shape) as raw text and
    /// deserialize into `T`. Plaintext analog of
    /// [`rustpass::Store::load_repo_config_as`] minus the unseal step. Used by
    /// each migration to read its own source-version snapshot.
    pub(crate) async fn read_app_json_as<T: serde::de::DeserializeOwned>(
        &self,
    ) -> Result<T, Error> {
        let s = fs::read_to_string(&self.app_json_path).await?;
        Ok(serde_json::from_str(&s)?)
    }

    /// Minimal raw read of the persisted schema version, for the migration
    /// engine's 3-state gate (see [`PeekOutcome`]). R074 moved `schema_version`
    /// from plaintext `pref.json` into the sealed merged `app.json`, so:
    /// - `pref.json` present (old world, schema < 8) ⇒ plaintext read ⇒ `Version`.
    /// - `pref.json` absent, `app.json` absent ⇒ `Absent` (fresh install).
    /// - `pref.json` absent, `app.json` present ⇒ unseal (the auth-free key is
    ///   loaded at `.setup()`, so this never `SEAL_KEY_UNAVAILABLE` in practice):
    ///   parses ⇒ `Version`; otherwise ⇒ `Corrupt` (halt, never silently skip).
    pub(crate) async fn peek_schema_version(&self) -> PeekOutcome {
        // OLD WORLD (pref.json present) — schema_version lives there as plaintext.
        if fs::try_exists(&self.pref_path).await.unwrap_or(false)
            && let Ok(s) = fs::read_to_string(&self.pref_path).await
            && let Ok(p) = serde_json::from_str::<SchemaVersionPeek>(&s)
        {
            return PeekOutcome::Version(p.schema_version);
        }
        // NEW WORLD (pref.json absent) — schema_version lives in the sealed merged
        // app.json. No app.json at all ⇒ fresh install ⇒ Absent.
        if !fs::try_exists(&self.app_json_path).await.unwrap_or(false) {
            return PeekOutcome::Absent;
        }
        // app.json present ⇒ unseal + parse schema_version. The Store is bound by
        // the time the engine calls this (init_state set_store first), and the
        // auth-free key is loaded at .setup() (decision D), so this unseals.
        let Some(store) = self.store.get() else {
            // Defensive: peek before set_store. A present-but-unreadable file is
            // Corrupt, not Absent — never silently skip a present config.
            return PeekOutcome::Corrupt;
        };
        match store.load_app_config().await {
            Ok(bytes) => match serde_json::from_slice::<SchemaVersionPeek>(&bytes) {
                Ok(p) => PeekOutcome::Version(p.schema_version),
                Err(_) => PeekOutcome::Corrupt,
            },
            // Absent slot (no app.json after the exists() race window) ⇒ skip.
            Err(e) if e.code == "NO_IDENTITY" => PeekOutcome::Absent,
            // Any other failure (tamper, key unexpectedly unavailable) on a
            // present file ⇒ Corrupt ⇒ halt + log (never silently Absent).
            Err(_) => PeekOutcome::Corrupt,
        }
    }

    /// Atomic temp+rename write of any serializable snapshot shape, mirroring
    /// [`AppConfigStore::save_pref`] WITHOUT the in-memory cache swap. Used by
    /// the migrations to write their target-version snapshot; the cache is
    /// re-read from disk once the whole chain finishes (see
    /// [`crate::migrations::run_app_migrations`]).
    pub(crate) async fn write_app_json_raw<T: Serialize>(&self, cfg: &T) -> Result<(), Error> {
        let json = serde_json::to_string_pretty(cfg)?;
        save_atomic(&self.app_json_path, json.as_bytes()).await
    }

    /// Re-read the on-disk config into the cache after the migration chain has
    /// written fresh files. Called by the engine at the end of a COMPLETED chain
    /// (`run_app_migrations`) and by `.setup()` to load the sealed config before
    /// first paint.
    ///
    /// R074 dual-world:
    /// - **Old world** (`pref.json` present, schema < 8): refresh the pref half
    ///   of the cache from `pref.json` (strict), then [`reload_behavior`] loads
    ///   the sealed behavior slot.
    /// - **New world** (`pref.json` absent, schema 8): the pref refresh is
    ///   skipped (no `pref.json`) and [`reload_behavior`] loads the single sealed
    ///   merged `app.json` into the cache.
    ///
    /// The pref.json parse error is propagated (the chain wrote a valid file; a
    /// reload failure is worth surfacing); `reload_behavior` soft-fails its half.
    /// The engine log+warns on a reload error rather than propagating further.
    pub(crate) async fn reload(&self) -> Result<(), Error> {
        // OLD WORLD: refresh the pref half of the cache from pref.json
        // (recomposing via the inverses so the behavior half is preserved).
        // Skipped in the new world (no pref.json — reload_behavior loads the
        // merged file).
        if fs::try_exists(&self.pref_path).await.unwrap_or(false) {
            let s = fs::read_to_string(&self.pref_path).await?;
            let pref: PrefConfig = serde_json::from_str(&s)?;
            let mut g = self.config.lock().expect("config lock poisoned");
            let behavior = BehaviorConfig::from_app(&g);
            *g = AppConfig::from_halves(&pref, &behavior);
        }
        // Behavior (old world: sealed slot) or the merged file (new world).
        self.reload_behavior().await
    }

    /// Snapshot the display half of the cache as a [`PrefConfig`] projection.
    pub(crate) fn get_pref(&self) -> PrefConfig {
        PrefConfig::from_app(&self.config.lock().expect("config lock poisoned"))
    }

    /// Snapshot the behavior half of the cache as a [`BehaviorConfig`]
    /// projection. The behavior half starts at default at construction;
    /// populate it via [`reload_behavior`](Self::reload_behavior) post-unlock.
    pub(crate) fn get_behavior(&self) -> BehaviorConfig {
        BehaviorConfig::from_app(&self.config.lock().expect("config lock poisoned"))
    }

    /// Snapshot the background-sync cadence (readable pre-unlock via the
    /// auth-free key, so the headless worker and a cold-start under `AppLock`
    /// can read it). (Used in the Android-only scheduling path; `dead_code` on
    /// desktop targets.)
    #[allow(dead_code)]
    pub(crate) fn background_sync(&self) -> BackgroundSyncCadence {
        self.config
            .lock()
            .expect("config lock poisoned")
            .background_sync
    }

    /// Clone the single merged [`AppConfig`] cache (the IPC view). This is what
    /// `get_app_config`, `apply_security_caches`, and tests consume, and the
    /// single sealed `app.json` persists ([`Self::save_merged`]). One cache ⇒
    /// the clone is an atomic snapshot (no torn half-state).
    pub(crate) fn get(&self) -> AppConfig {
        self.config.lock().expect("config lock poisoned").clone()
    }

    /// Resolve the locale the app should render in: an explicit, supported
    /// override when one is set, otherwise the system locale (normalized to a
    /// supported code). Always returns a value in [`SUPPORTED_LOCALES`]. A
    /// stale/unsupported on-disk override (including `Some("")` from a
    /// hand-edited file) degrades to system-locale resolution rather than
    /// poisoning the result — the frontend therefore reads this, not the raw
    /// `locale` field, so an unsupported value never reaches the `WebView`.
    pub(crate) fn resolved_locale(&self) -> String {
        let pref = self.get_pref();
        match pref.locale.as_deref() {
            Some(explicit) if is_supported_locale(explicit) => explicit.to_string(),
            _ => normalize_system_locale(sys_locale::get_locale().as_deref()),
        }
    }

    /// Effective runtime log filter: `Debug` while a verbose deadline is set and
    /// not yet past, else `Info`. Lazy — an expired deadline reads as Info here
    /// without being cleared; the startup path calls [`Self::clear_expired_verbose`]
    /// to persist the revert. Reads the cache projection (`verbose_until`
    /// lives in the sealed merged `app.json`, readable pre-unlock via the
    /// auth-free key), so this is safe to call pre-unlock.
    #[must_use]
    pub(crate) fn effective_log_filter(&self) -> log::LevelFilter {
        match self.get_pref().verbose_until {
            Some(deadline) if deadline > now_unix() => log::LevelFilter::Debug,
            // None, expired, or a stale value all resolve to Info.
            _ => log::LevelFilter::Info,
        }
    }

    /// Turn verbose (Debug) logging on for [`VERBOSE_WINDOW_SECS`], or off. `on`
    /// stamps a fresh deadline measured from now (the window restarts, never
    /// extends); `off` clears it immediately. Persists + returns the updated
    /// config. The runtime gate + timer are re-applied by the caller (the
    /// `set_verbose` command) so the level changes within the current session.
    pub(crate) async fn set_verbose(&self, on: bool) -> Result<AppConfig, Error> {
        self.update(|c| c.verbose_until = on.then(|| now_unix() + VERBOSE_WINDOW_SECS))
            .await
    }

    /// Set the periodic background-sync cadence; persists the merged `app.json`
    /// and returns the merged [`AppConfig`].
    pub(crate) async fn set_background_sync(
        &self,
        cadence: BackgroundSyncCadence,
    ) -> Result<AppConfig, Error> {
        self.update(|c| c.background_sync = cadence).await
    }

    /// Path of the sync-attention marker file.
    fn sync_attention_marker_path(&self) -> PathBuf {
        self.pref_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(SYNC_ATTENTION_FILE)
    }

    /// Atomically create the attention marker (headless Worker, on divergence).
    /// No read-modify-write ⇒ no race with a concurrent foreground pref write.
    #[allow(dead_code)] // reached only on the headless-sync Ok path (android / divergence tests).
    pub(crate) async fn set_sync_attention_marker(&self) -> Result<(), Error> {
        save_atomic(&self.sync_attention_marker_path(), b"").await
    }

    /// Take-once: whether the marker existed, and remove it. Used by the
    /// foreground on cold-start to decide whether to trigger a sync.
    pub(crate) async fn consume_sync_attention_marker(&self) -> bool {
        fs::remove_file(self.sync_attention_marker_path())
            .await
            .is_ok()
    }

    /// Persist-clear an expired verbose deadline. Best-effort at startup: the
    /// level is already lazy-Info via [`Self::effective_log_filter`], so a failure
    /// here is non-fatal — the next launch retries. Re-checks expiry INSIDE the
    /// closure so a `set_verbose` landing between the read and the swap is not
    /// clobbered.
    pub(crate) async fn clear_expired_verbose(&self) -> Result<(), Error> {
        if self.get_pref().verbose_until.is_none() {
            return Ok(());
        }
        self.update(|c| {
            if c.verbose_until.is_some_and(|d| d <= now_unix()) {
                c.verbose_until = None;
            }
        })
        .await?;
        Ok(())
    }

    /// Persist `cfg` to `pref.json` atomically (via `rustpass::config::save_atomic`
    /// — temp + rename, DRY), then refresh the cache via [`reload`](Self::reload).
    ///
    /// Migration helper (`m0005`–`m0007`); the runtime uses [`update`](Self::update).
    /// The cache refresh re-reads `pref.json` (and the behavior slot when a Store
    /// is bound), so it stays coherent with disk without a half-swap.
    pub(crate) async fn save_pref(&self, cfg: &PrefConfig) -> Result<(), Error> {
        let json = serde_json::to_string_pretty(cfg)?;
        save_atomic(&self.pref_path, json.as_bytes()).await?;
        self.reload().await?;
        Ok(())
    }

    /// Serialize `cfg` to bytes and seal them into `app.json` via the bound
    /// Store's `save_app_behavior`, then refresh the cache via
    /// [`reload`](Self::reload). The Seal itself gates: passthrough on desktop
    /// (key `None`), `SealKeyUnavailable` if ever-keyed-then-wiped (the
    /// app-launch lock cold-start path). No separate `app_locked` reject — it
    /// would wrongly reject desktop.
    ///
    /// Migration helper (`m0005`/`m0006`); the runtime uses [`update`](Self::update).
    pub(crate) async fn save_behavior(&self, cfg: &BehaviorConfig) -> Result<(), Error> {
        let json = serde_json::to_string_pretty(cfg)?;
        let bytes = json.into_bytes();
        let store = self.store.get().ok_or_else(|| {
            Error::new(
                ErrorCode::ConfigError,
                "AppConfigStore: Store not bound (call set_store first)",
            )
        })?;
        store.save_app_behavior(&bytes).await?;
        self.reload().await?;
        Ok(())
    }

    /// Read + unseal `app.json` and refresh the behavior half of the cache.
    /// Soft-fails on `NoIdentity` (missing slot, pre-unlock) and
    /// `SealKeyUnavailable` (master key not yet injected) — both are normal
    /// pre-unlock states, not errors. Mirrors `new()`'s resilience on
    /// parse/IO errors (warn + leave the cache at the last-read value).
    ///
    /// R074: when `pref.json` is absent (schema 8, the post-collapse single-file
    /// world), behavior lives in the merged sealed `app.json` alongside the
    /// display prefs — so this loads the whole merged file into the cache
    /// (the inverse of [`Self::save_merged`]).
    pub(crate) async fn reload_behavior(&self) -> Result<(), Error> {
        let Some(store) = self.store.get() else {
            // No Store bound — nothing to load. Leave the cache at defaults.
            return Ok(());
        };
        // NEW WORLD (pref.json absent): behavior lives in the merged sealed file.
        if !self.pref_json_exists().await {
            return self.reload_merged(store).await;
        }
        // OLD WORLD: behavior slot only (sealed `app_behavior`-tagged slot).
        match store.load_app_behavior().await {
            Ok(bytes) => match serde_json::from_slice::<BehaviorConfig>(&bytes) {
                Ok(cfg) => {
                    let mut g = self.config.lock().expect("config lock poisoned");
                    let pref = PrefConfig::from_app(&g);
                    *g = AppConfig::from_halves(&pref, &cfg);
                    Ok(())
                }
                Err(e) => {
                    log::warn!("app-config: app.json behavior unparseable, leaving the cache: {e}");
                    Ok(())
                }
            },
            Err(e) if e.code == "NO_IDENTITY" => Ok(()),
            Err(e) if e.code == "SEAL_KEY_UNAVAILABLE" => Ok(()),
            Err(e) => {
                log::warn!("app-config: app.json behavior load failed, leaving the cache: {e}");
                Ok(())
            }
        }
    }

    /// Load the merged sealed `app.json` (dual-AAD read) and set the whole
    /// cache. Soft-fails on a missing slot / unavailable key / parse error
    /// (mirrors [`Self::reload_behavior`]'s resilience). Used by the new-world
    /// branch of [`Self::reload_behavior`] and the post-migration [`Self::reload`].
    async fn reload_merged(&self, store: &Arc<Store>) -> Result<(), Error> {
        match store.load_app_config().await {
            Ok(bytes) => match serde_json::from_slice::<AppConfig>(&bytes) {
                Ok(cfg) => {
                    *self.config.lock().expect("config lock poisoned") = cfg;
                    Ok(())
                }
                Err(e) => {
                    log::warn!("app-config: merged app.json unparseable, leaving the cache: {e}");
                    Ok(())
                }
            },
            Err(e) if e.code == "NO_IDENTITY" => Ok(()),
            Err(e) if e.code == "SEAL_KEY_UNAVAILABLE" => Ok(()),
            Err(e) => {
                log::warn!("app-config: merged app.json load failed, leaving the cache: {e}");
                Ok(())
            }
        }
    }

    /// Persist the merged `cfg` as the single sealed `app.json` — the R074
    /// post-collapse runtime write path (replaces the split pref.json +
    /// behavior-slot writes for schema 8+). Does NOT swap the cache: the caller
    /// ([`update`](Self::update)) swaps the whole cache after this succeeds, so a
    /// write failure leaves the cache consistent with disk (write-then-swap). The
    /// Seal gates: passthrough on desktop, `SealKeyUnavailable` if
    /// ever-keyed-then-wiped.
    async fn save_merged(&self, cfg: &AppConfig) -> Result<(), Error> {
        let json = serde_json::to_string_pretty(cfg)?;
        let bytes = json.into_bytes();
        let store = self.store.get().ok_or_else(|| {
            Error::new(
                ErrorCode::ConfigError,
                "AppConfigStore: Store not bound (call set_store first)",
            )
        })?;
        store.save_app_config(&bytes).await
    }

    /// Persist `cfg` as a plaintext legacy single-file `app.json` (all fields)
    /// via `save_atomic` and set the whole cache to mirror the write. Test-only —
    /// the PRE-SPLIT persistence path, used by legacy round-trip tests to seed
    /// a pre-split file shape. Production writes go through
    /// [`save_pref`](Self::save_pref) / [`save_behavior`](Self::save_behavior)
    /// (post-split) or [`write_app_json_raw`](Self::write_app_json_raw) (the
    /// migration chain itself, which writes raw with no cache swap so the
    /// end-of-chain reload is the single source of truth).
    #[cfg(test)]
    pub(crate) async fn save_legacy_app_json(&self, cfg: &LegacyAppConfig) -> Result<(), Error> {
        let json = serde_json::to_string_pretty(cfg)?;
        save_atomic(&self.app_json_path, json.as_bytes()).await?;
        // Keep the cache in sync with the legacy write so a subsequent get()
        // reflects the new value without a round-trip through disk.
        *self.config.lock().expect("config lock poisoned") = AppConfig::from_halves(
            &PrefConfig::from_legacy(cfg),
            &BehaviorConfig::from_legacy(cfg),
        );
        Ok(())
    }

    /// The single runtime read-modify-write helper (R074): every setter clones
    /// the [`AppConfig`] cache, mutates it, persists it to the sealed `app.json`
    /// via [`Self::save_merged`], then swaps the whole cache back. This replaces
    /// the old split-era `update_pref`/`update_behavior` pair — one merged file,
    /// one cache, one helper. No merge-on-read, no split-on-write.
    ///
    /// The `write_mu` async mutex is held across the whole read-modify-write
    /// (including the sealed-write `.await`): without serialization two
    /// concurrent setters could each clone the same cache and the second save
    /// would overwrite the first. An async mutex (not `std::sync::Mutex`)
    /// because it spans the `.await`. The std `config` mutex is held only for
    /// the quick clone/swap, never across an `.await`.
    async fn update<F: FnOnce(&mut AppConfig)>(&self, f: F) -> Result<AppConfig, Error> {
        let _guard = self.write_mu.lock().await;
        let mut cfg = self.config.lock().expect("config lock poisoned").clone();
        f(&mut cfg);
        self.save_merged(&cfg).await?;
        // Swap the whole persisted config back so a later get() reflects it
        // without a round-trip through disk.
        *self.config.lock().expect("config lock poisoned") = cfg.clone();
        Ok(cfg)
    }

    /// Set the auto-lock mode (sealed). `Idle(n)` is clamped first.
    pub(crate) async fn set_lock_mode(&self, mode: LockMode) -> Result<AppConfig, Error> {
        self.update(|c| c.lock_mode = clamp_lock_mode(mode)).await
    }

    /// Set the password-view auto-clear override (sealed). `None` ⇒ default,
    /// `Some(0)` ⇒ never, else clamped to the allowed range.
    pub(crate) async fn set_view_clear_secs(&self, secs: Option<u64>) -> Result<AppConfig, Error> {
        self.update(|c| c.view_clear_secs = normalize_clear_secs(secs))
            .await
    }

    /// Set the clipboard auto-clear override (sealed, same rule as view-clear).
    pub(crate) async fn set_clipboard_clear_secs(
        &self,
        secs: Option<u64>,
    ) -> Result<AppConfig, Error> {
        self.update(|c| c.clipboard_clear_secs = normalize_clear_secs(secs))
            .await
    }

    /// Set the per-save autosync flag (sealed).
    pub(crate) async fn set_autosync(&self, enabled: bool) -> Result<AppConfig, Error> {
        self.update(|c| c.autosync = enabled).await
    }

    /// Set the persisted app-launch biometric-gate intent flag (sealed;
    /// write-only mirror of the Keystore-probed runtime state).
    pub(crate) async fn set_biometric_app_lock(&self, enabled: bool) -> Result<AppConfig, Error> {
        self.update(|c| c.biometric_app_lock = enabled).await
    }

    /// Set the app-launch-gate in-app idle timeout (sealed). `After(n)` is
    /// clamped to the preset range first. The Tauri `set_gate_idle` command
    /// applies the new value to the live backend timer (R057); this store method
    /// only persists + returns the updated config.
    pub(crate) async fn set_gate_idle(&self, mode: GateIdle) -> Result<AppConfig, Error> {
        self.update(|c| c.gate_idle = clamp_gate_idle(mode)).await
    }

    /// Set the persisted color-scheme override (`None` ⇒ track system). `Some`
    /// must be one of [`SUPPORTED_THEME_MODES`]; a bad value returns
    /// `ConfigError`. The frontend applies the runtime effect (the `data-theme`
    /// attribute) on receipt, so this stays a pure persistence step mirroring
    /// `set_locale`.
    pub(crate) async fn set_theme_mode(&self, mode: Option<String>) -> Result<AppConfig, Error> {
        validate_theme_mode(mode.as_deref())?;
        self.update(|c| c.theme_mode = mode).await
    }

    /// Set the display-language preference (`null` clears the override — track
    /// system; `"en"` / `"zh-CN"` pin it). Mirrors `set_theme_mode`. The frontend
    /// re-applies the locale on receipt.
    pub(crate) async fn set_locale(&self, locale: Option<String>) -> Result<AppConfig, Error> {
        validate_locale(locale.as_deref())?;
        self.update(|c| c.locale = locale).await
    }

    /// Set the persisted three-state screen-capture mode (sealed). Rejects
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
        self.update(|c| c.secure_screen_mode = Some(mode)).await
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

/// Read the merged app config (the IPC view).
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
    state.app_config.set_locale(locale).await
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

/// The authoritative locale the app should render in. The frontend reconciles
/// against the value the `.setup()` init script already baked in pre-paint
/// (which carries the resolved — pinned-or-system — locale); this is the
/// post-mount safety net for the rare case where the sealed config was
/// unreadable at setup.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn resolved_locale(state: State<'_, AppState>) -> String {
    state.app_config.resolved_locale()
}

/// The effective diagnostics log level (persisted value or `"info"` default).
/// Turn verbose (Debug) logging on for a bounded window, or off. Returns the
/// updated config and re-applies the runtime log gate so the level takes effect
/// immediately. On enable, `revert_notify` stages the localized text the deadline
/// timer posts as an OS notification when the window elapses (so the notice fires
/// even if the `WebView` is backgrounded). Verbose persists; the deadline
/// auto-reverts to Info when it elapses (mid-session via the timer, or at the
/// next launch if the process was killed). See RFC 0055.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn set_verbose(
    state: State<'_, AppState>,
    app: AppHandle,
    enabled: bool,
    revert_notify: Option<VerboseNotifyText>,
) -> Result<AppConfig, Error> {
    let cfg = state.app_config.set_verbose(enabled).await?;
    // Re-apply the runtime gate so the level changes within this session, not
    // just on the next launch.
    log::set_max_level(state.app_config.effective_log_filter());
    if enabled {
        // Stage the revert-notification text + arm the mid-session revert timer.
        state.app_config.set_revert_notify(revert_notify);
        arm_verbose_timer(&state, &app);
    } else {
        // Cancel any in-flight revert + drop the staged text.
        state.app_config.set_revert_notify(None);
        disarm_verbose_timer(&state);
    }
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    async fn store_at(dir: &Path) -> AppConfigStore {
        AppConfigStore::new(dir).await
    }

    /// Bind a desktop-passthrough Store (`master_key = None`) so the sealed
    /// behavior setters/readers can flow. The seal is plaintext-passthrough in
    /// this mode, so behavior round-trips through `app.json` as plaintext JSON.
    async fn store_with_desktop_store(dir: &Path) -> AppConfigStore {
        let s = AppConfigStore::new(dir).await;
        s.set_store(Arc::new(Store::new(dir.to_path_buf(), None)));
        s.reload_behavior().await.ok();
        s
    }

    #[tokio::test]
    async fn missing_file_defaults_sensitive_mode() {
        // A missing app.json (fresh install) loads defaults — secure_screen_mode
        // is None, which the frontend resolves to the Sensitive default.
        let dir = tempdir().expect("tempdir");
        assert!(
            store_at(dir.path())
                .await
                .get()
                .secure_screen_mode
                .is_none(),
            "missing app.json must fall back to the default, not panic"
        );
    }

    #[tokio::test]
    async fn corrupt_file_defaults_sensitive_mode() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join(APP_CONFIG_FILE), "{not json").unwrap();
        assert!(
            store_at(dir.path())
                .await
                .get()
                .secure_screen_mode
                .is_none(),
            "corrupt app.json must fall back to the default, not panic"
        );
    }

    #[tokio::test]
    async fn default_locale_is_none() {
        assert!(AppConfig::default().locale.is_none());
    }

    #[tokio::test]
    async fn locale_roundtrips_through_save() {
        let dir = tempdir().expect("tempdir");
        let store = store_at(dir.path()).await;
        store
            .save_legacy_app_json(&LegacyAppConfig {
                locale: Some("zh-CN".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let reloaded = store_at(dir.path()).await.get();
        assert_eq!(reloaded.locale.as_deref(), Some("zh-CN"));
    }

    #[tokio::test]
    async fn locale_omitted_on_disk_when_none() {
        // skip_serializing_if keeps the field out of the file when it is None,
        // so existing files stay byte-identical and don't carry a null.
        let dir = tempdir().expect("tempdir");
        let store = store_at(dir.path()).await;
        store
            .save_legacy_app_json(&LegacyAppConfig {
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

    #[tokio::test]
    async fn existing_app_json_without_locale_loads() {
        // An app.json written before the locale field existed must still parse,
        // with locale defaulting to None (backward compatibility).
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join(APP_CONFIG_FILE), "{}").unwrap();
        assert!(store_at(dir.path()).await.get().locale.is_none());
    }

    #[tokio::test]
    async fn validate_locale_accepts_supported_and_none() {
        assert!(validate_locale(None).is_ok());
        assert!(validate_locale(Some("en")).is_ok());
        assert!(validate_locale(Some("zh-CN")).is_ok());
    }

    #[tokio::test]
    async fn validate_locale_rejects_unknown() {
        let err = validate_locale(Some("zh-TW")).unwrap_err();
        assert_eq!(err.code, "CONFIG_ERROR");
        assert!(err.message.contains("zh-TW"));
        assert!(validate_locale(Some("fr")).is_err());
    }

    #[tokio::test]
    async fn default_theme_mode_is_none() {
        assert!(AppConfig::default().theme_mode.is_none());
    }

    #[tokio::test]
    async fn theme_mode_roundtrips_through_save() {
        let dir = tempdir().expect("tempdir");
        let store = store_at(dir.path()).await;
        store
            .save_legacy_app_json(&LegacyAppConfig {
                theme_mode: Some("dark".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let reloaded = store_at(dir.path()).await.get();
        assert_eq!(reloaded.theme_mode.as_deref(), Some("dark"));
    }

    #[tokio::test]
    async fn theme_mode_omitted_on_disk_when_none() {
        // skip_serializing_if keeps theme_mode out of app.json when None, so
        // existing files stay byte-identical and carry no null.
        let dir = tempdir().expect("tempdir");
        let store = store_at(dir.path()).await;
        store
            .save_legacy_app_json(&LegacyAppConfig {
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

    #[tokio::test]
    async fn existing_app_json_without_theme_mode_loads() {
        // An app.json written before theme_mode existed must still parse, with
        // theme_mode defaulting to None (backward compatibility — adding the
        // optional field is non-breaking, like locale).
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join(APP_CONFIG_FILE), "{}").unwrap();
        assert!(store_at(dir.path()).await.get().theme_mode.is_none());
    }

    #[tokio::test]
    async fn set_theme_mode_persists_validates_and_clears() {
        let dir = tempdir().expect("tempdir");
        // R074: runtime setters persist the merged sealed app.json, so a Store
        // must be bound (desktop passthrough).
        let store = store_with_desktop_store(dir.path()).await;
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

    /// R074: a runtime setter (`update`) persists the whole merged `AppConfig`,
    /// so the behavior half must survive a later display-pref write (and vice
    /// versa). One cache carries both halves, so this pins that a setter doesn't
    /// clobber the half it didn't touch.
    #[tokio::test]
    async fn runtime_pref_setter_preserves_behavior_half() {
        let dir = tempdir().expect("tempdir");
        let store = store_with_desktop_store(dir.path()).await;
        // A behavior value, then a display value — two separate merged writes.
        store.set_lock_mode(LockMode::Idle(300)).await.unwrap();
        store.set_locale(Some("zh-CN".to_string())).await.unwrap();
        // Re-read from disk (fresh store + reload): BOTH halves must persist.
        let reloaded = store_with_desktop_store(dir.path()).await.get();
        assert_eq!(reloaded.locale.as_deref(), Some("zh-CN"));
        assert_eq!(
            reloaded.lock_mode,
            LockMode::Idle(300),
            "the behavior half survives the later display-pref merged write"
        );
    }

    /// a migration-helper sequence (mirroring m0005's `save_pref` →
    /// `save_behavior` order) keeps both halves of the single cache coherent —
    /// each write+reload re-reads the other half from disk rather than
    /// clobbering it. Pins the write+reload path.
    #[tokio::test]
    async fn save_behavior_then_save_pref_keeps_both_halves_coherent() {
        let dir = tempdir().expect("tempdir");
        let store = store_with_desktop_store(dir.path()).await;
        // save_behavior (behavior half) then save_pref (display half): the
        // display write+reload must not wipe the just-sealed behavior half.
        store
            .save_behavior(&BehaviorConfig {
                lock_mode: LockMode::Idle(300),
                ..BehaviorConfig::default()
            })
            .await
            .unwrap();
        store
            .save_pref(&PrefConfig {
                locale: Some("zh-CN".to_string()),
                ..PrefConfig::default()
            })
            .await
            .unwrap();
        assert_eq!(store.get_pref().locale.as_deref(), Some("zh-CN"));
        assert_eq!(
            store.get_behavior().lock_mode,
            LockMode::Idle(300),
            "the behavior half survives the later save_pref + reload"
        );
    }

    /// Reverse direction: `save_pref` then `save_behavior` keeps the
    /// display half coherent — the behavior write+reload must not clobber the
    /// just-written display half.
    #[tokio::test]
    async fn save_pref_then_save_behavior_keeps_display_half_coherent() {
        let dir = tempdir().expect("tempdir");
        let store = store_with_desktop_store(dir.path()).await;
        store
            .save_pref(&PrefConfig {
                locale: Some("zh-CN".to_string()),
                ..PrefConfig::default()
            })
            .await
            .unwrap();
        store
            .save_behavior(&BehaviorConfig {
                lock_mode: LockMode::Idle(300),
                ..BehaviorConfig::default()
            })
            .await
            .unwrap();
        assert_eq!(store.get_behavior().lock_mode, LockMode::Idle(300));
        assert_eq!(
            store.get_pref().locale.as_deref(),
            Some("zh-CN"),
            "the display half survives the later save_behavior + reload"
        );
    }

    /// Cold-start locale availability (new-world analog of the
    /// cold-start-biometricprompt-locale prior learning). On Android the merged
    /// `app.json` is a real seal envelope, so `new()` cannot lift it (the legacy
    /// lift fails on the binary envelope) and the cache starts at the default.
    /// The pinned locale is only restored by the subsequent `reload()` (the
    /// auth-free key, loaded at `.setup()` per decision D). Pins the startup
    /// ordering lib.rs relies on: `reload()` MUST run before `resolved_locale()`
    /// at first paint.
    #[tokio::test]
    async fn reload_restores_pinned_locale_when_new_cannot_lift_envelope() {
        let dir = tempdir().expect("tempdir");
        let key = rustpass::seal::generate_master_key().unwrap();
        let store = Arc::new(Store::new(dir.path().to_path_buf(), Some(key)));
        // Seed the merged sealed app.json with a pinned locale (a real envelope).
        store
            .save_app_config(r#"{"schema_version":8,"locale":"zh-CN"}"#.as_bytes())
            .await
            .unwrap();
        assert!(
            rustpass::seal::is_envelope(&std::fs::read(dir.path().join(APP_CONFIG_FILE)).unwrap()),
            "precondition: merged app.json is a real envelope"
        );
        // `new()` cannot lift a real envelope as LegacyAppConfig — cache default.
        let ac = AppConfigStore::new(dir.path()).await;
        assert!(
            ac.get_pref().locale.is_none(),
            "new() must fall to default when the merged file is a real envelope"
        );
        // Bind + reload mirrors lib.rs startup: the auth-free key unseals the
        // merged file, restoring the pinned locale before first paint.
        ac.set_store(store);
        ac.reload().await.unwrap();
        assert_eq!(
            ac.resolved_locale(),
            "zh-CN",
            "reload() must restore the pinned locale after the default-only new()"
        );
    }

    /// `update()`'s `write_mu` serializes concurrent setters so a second save
    /// can't overwrite the first (last-write-wins would lose an update). Without
    /// the mutex, tasks that each clone the cache, mutate a distinct field, and
    /// save would leave only the last task's field on disk.
    #[tokio::test]
    async fn concurrent_update_setters_dont_lose_updates() {
        let dir = tempdir().expect("tempdir");
        let store = Arc::new(store_with_desktop_store(dir.path()).await);
        // Four concurrent setters, each on a distinct field.
        let s = Arc::clone(&store);
        let h1 = tokio::spawn(async move { s.set_locale(Some("zh-CN".to_string())).await });
        let s = Arc::clone(&store);
        let h2 = tokio::spawn(async move { s.set_lock_mode(LockMode::Idle(300)).await });
        let s = Arc::clone(&store);
        let h3 = tokio::spawn(async move { s.set_autosync(false).await });
        let s = Arc::clone(&store);
        let h4 = tokio::spawn(async move { s.set_theme_mode(Some("dark".to_string())).await });
        h1.await.unwrap().unwrap();
        h2.await.unwrap().unwrap();
        h3.await.unwrap().unwrap();
        h4.await.unwrap().unwrap();
        // Re-read off disk: every field must survive (no lost update).
        let reloaded = store_with_desktop_store(dir.path()).await.get();
        assert_eq!(reloaded.locale.as_deref(), Some("zh-CN"));
        assert_eq!(reloaded.lock_mode, LockMode::Idle(300));
        assert!(!reloaded.autosync);
        assert_eq!(reloaded.theme_mode.as_deref(), Some("dark"));
    }

    /// `reload_merged` soft-fails on an unparseable merged `app.json` (warn +
    /// leave the cache at the prior value), mirroring `reload_behavior`'s
    /// resilience. Pins the new-world parse-error path.
    #[tokio::test]
    async fn reload_merged_leaves_cache_on_unparseable_merged_app_json() {
        let dir = tempdir().expect("tempdir");
        let store = Arc::new(Store::new(dir.path().to_path_buf(), None));
        // Seed a valid merged app.json with a pinned locale (desktop passthrough
        // => the file is plaintext, so raw bytes are fine).
        store
            .save_app_config(r#"{"schema_version":8,"locale":"zh-CN"}"#.as_bytes())
            .await
            .unwrap();
        let ac = AppConfigStore::new(dir.path()).await;
        ac.set_store(store);
        ac.reload().await.unwrap();
        assert_eq!(ac.get_pref().locale.as_deref(), Some("zh-CN"));
        // Corrupt the merged file. The next reload must NOT panic and must leave
        // the cache at the prior value (zh-CN), not revert to default.
        std::fs::write(dir.path().join(APP_CONFIG_FILE), "{not json").unwrap();
        ac.reload().await.unwrap();
        assert_eq!(
            ac.get_pref().locale.as_deref(),
            Some("zh-CN"),
            "an unparseable merged file leaves the cache at the prior value"
        );
    }

    #[tokio::test]
    async fn app_config_store_new_missing_file_uses_defaults() {
        let dir = tempdir().expect("tempdir");
        let store = AppConfigStore::new(dir.path()).await;
        assert_eq!(
            store.get().schema_version,
            AppConfig::default().schema_version,
            "missing app.json must fall back to the default (current schema target)"
        );
    }

    #[tokio::test]
    async fn app_config_store_new_corrupt_json_uses_defaults() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join(APP_CONFIG_FILE), "{not valid json").unwrap();
        let store = AppConfigStore::new(dir.path()).await;
        assert_eq!(
            store.get().schema_version,
            AppConfig::default().schema_version,
            "corrupt app.json must fall back to the default, not panic"
        );
    }

    #[tokio::test]
    async fn app_config_store_new_valid_file_loads_value() {
        let dir = tempdir().expect("tempdir");
        // A non-default value round-trips: secure_screen_mode "off" (default is
        // None / Sensitive).
        std::fs::write(
            dir.path().join(APP_CONFIG_FILE),
            serde_json::json!({ "secure_screen_mode": "off" }).to_string(),
        )
        .unwrap();
        let store = AppConfigStore::new(dir.path()).await;
        assert_eq!(
            store.get().secure_screen_mode,
            Some(SecureScreenMode::Off),
            "a valid file's secure_screen_mode must load (not revert to default)"
        );
    }

    #[tokio::test]
    async fn verbose_until_roundtrips_through_pref() {
        let dir = tempdir().expect("tempdir");
        let store = store_at(dir.path()).await;
        let pinned = now_unix() + 42;
        store
            .save_pref(&PrefConfig {
                verbose_until: Some(pinned),
                ..PrefConfig::default()
            })
            .await
            .unwrap();
        let reloaded = store_at(dir.path()).await.get_pref();
        assert_eq!(reloaded.verbose_until, Some(pinned));
    }

    #[tokio::test]
    async fn verbose_until_omitted_on_disk_when_none() {
        // skip_serializing_if keeps verbose_until out of pref.json while None,
        // so a default config stays byte-identical.
        let dir = tempdir().expect("tempdir");
        let store = store_at(dir.path()).await;
        store.save_pref(&PrefConfig::default()).await.unwrap();
        let on_disk = std::fs::read_to_string(dir.path().join(PREF_FILE)).unwrap();
        assert!(
            !on_disk.contains("verbose_until"),
            "verbose_until key must be absent when None; got: {on_disk}"
        );
    }

    #[tokio::test]
    async fn effective_log_filter_reflects_verbose_deadline() {
        let dir = tempdir().expect("tempdir");
        // R074: set_verbose persists the merged sealed app.json (Store bound).
        let store = store_with_desktop_store(dir.path()).await;
        // No deadline ⇒ Info.
        assert_eq!(store.effective_log_filter(), log::LevelFilter::Info);
        // A fresh verbose window ⇒ Debug.
        store.set_verbose(true).await.unwrap();
        assert_eq!(store.effective_log_filter(), log::LevelFilter::Debug);
        // Cleared ⇒ Info again.
        store.set_verbose(false).await.unwrap();
        assert_eq!(store.effective_log_filter(), log::LevelFilter::Info);
    }

    #[tokio::test]
    async fn clear_expired_verbose_reverts_a_past_deadline() {
        let dir = tempdir().expect("tempdir");
        // R074: clear_expired_verbose persists the merged sealed app.json.
        let store = store_with_desktop_store(dir.path()).await;
        // Stamp a deadline already in the past.
        store
            .save_pref(&PrefConfig {
                verbose_until: Some(now_unix().saturating_sub(60)),
                ..PrefConfig::default()
            })
            .await
            .unwrap();
        assert_eq!(
            store.effective_log_filter(),
            log::LevelFilter::Info,
            "an expired deadline reads as Info"
        );
        store.clear_expired_verbose().await.unwrap();
        assert!(
            store.get_pref().verbose_until.is_none(),
            "an expired deadline is cleared off disk"
        );
    }

    #[tokio::test]
    async fn clear_expired_verbose_leaves_a_live_window_alone() {
        let dir = tempdir().expect("tempdir");
        // R074: set_verbose/clear_expired_verbose persist the merged sealed app.json.
        let store = store_with_desktop_store(dir.path()).await;
        store.set_verbose(true).await.unwrap();
        let live = store.get_pref().verbose_until;
        store.clear_expired_verbose().await.unwrap();
        assert_eq!(
            store.get_pref().verbose_until,
            live,
            "a live verbose window is not cleared"
        );
    }

    #[tokio::test]
    async fn normalize_system_locale_maps_variants() {
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
        let store = store_at(dir.path()).await;
        store
            .save_legacy_app_json(&LegacyAppConfig {
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
        let store = store_at(dir.path()).await;
        store
            .save_legacy_app_json(&LegacyAppConfig {
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

    #[tokio::test]
    async fn resolved_locale_with_none_returns_supported() {
        let dir = tempdir().expect("tempdir");
        let resolved = store_at(dir.path()).await.resolved_locale();
        assert!(
            is_supported_locale(&resolved),
            "resolved locale must be supported, got {resolved}"
        );
    }

    #[test]
    fn locale_init_script_emits_canonical_locale_assignment() {
        // The resolved locale bakes both window.__GPM_LOCALE__ AND <html lang>
        // into the pre-paint init script so resolveBootLocale() reads the pinned
        // (or system-fallback) locale at module load and a screen reader gets
        // the right pronunciation from frame 0. Exact-match locks the format!
        // shape (a stray/missing brace, quote, or `;` fails).
        assert_eq!(
            locale_init_script("zh-CN"),
            r#"(function(){window.__GPM_LOCALE__ = "zh-CN";var d=document.documentElement;if(d){d.lang="zh-CN";}})();"#
        );
        assert_eq!(
            locale_init_script("en"),
            r#"(function(){window.__GPM_LOCALE__ = "en";var d=document.documentElement;if(d){d.lang="en";}})();"#
        );
    }

    #[test]
    fn locale_init_script_escapes_html_script_close_tag() {
        // A `</script>` in a planted locale must not break out of the HTML
        // <script> Tauri injects on older Android WebViews. resolved_locale only
        // ever returns en/zh-CN, so this is purely defensive — mirroring the
        // theme guard (prior learning: tauri-init-script-script-breakout). The
        // escape lives in the shared init_script_json.
        let payload = "</script><script>alert(1)</script>";
        let s = locale_init_script(payload);
        assert!(
            !s.contains("</script"),
            "raw </script must not survive, got: {s}"
        );
        assert!(s.contains("<\\/script"), "must escape </ as <\\/, got: {s}");
    }

    #[test]
    fn theme_init_script_pins_data_theme_for_light_and_dark() {
        // A pinned value bakes `m="<value>"` into the pre-paint init script and
        // sets `data-theme` to it, so the `:root[data-theme="..."]` rule (and
        // `color-scheme`) apply from frame 0 — no cold-start flash. Exact-match
        // locks in the `format!` brace-escaping (a stray/missing brace fails).
        assert_eq!(
            theme_init_script(Some("light")),
            "(function(){var m=\"light\",d=document.documentElement;if(d){if(m==='light'||m==='dark'){d.setAttribute('data-theme',m);}else{d.removeAttribute('data-theme');}}})();"
        );
        assert_eq!(
            theme_init_script(Some("dark")),
            "(function(){var m=\"dark\",d=document.documentElement;if(d){if(m==='light'||m==='dark'){d.setAttribute('data-theme',m);}else{d.removeAttribute('data-theme');}}})();"
        );
    }

    #[test]
    fn theme_init_script_clears_data_theme_for_system_and_garbage() {
        // `None` (track system) clears the attribute (defensive; it starts
        // absent on a fresh document). A garbage value still emits the
        // `setAttribute` call, but the `if (m === 'light' || m === 'dark')`
        // guard routes it to `removeAttribute` — degrading to system instead
        // of poisoning frame 0, mirroring `normalize_theme_mode`.
        assert_eq!(
            theme_init_script(None),
            "(function(){var m=null,d=document.documentElement;if(d){if(m==='light'||m==='dark'){d.setAttribute('data-theme',m);}else{d.removeAttribute('data-theme');}}})();"
        );
        assert_eq!(
            theme_init_script(Some("blue")),
            "(function(){var m=\"blue\",d=document.documentElement;if(d){if(m==='light'||m==='dark'){d.setAttribute('data-theme',m);}else{d.removeAttribute('data-theme');}}})();"
        );
    }

    #[test]
    fn theme_init_script_escapes_html_script_close_tag() {
        // A `</script>` in a planted theme_mode must not break out of the HTML
        // <script> Tauri injects on older Android WebViews. serde_json doesn't
        // escape `/`, so the function escapes `</` to `<\/` itself (JSON-valid,
        // JS-equivalent, HTML-safe — `<\/script>` won't match the end-tag rule).
        let payload = "</script><script>alert(1)</script>";
        let s = theme_init_script(Some(payload));
        assert!(
            !s.contains("</script"),
            "raw </script must not survive into the inject, got: {s}"
        );
        assert!(s.contains("<\\/script"), "must escape </ as <\\/, got: {s}");
    }

    #[tokio::test]
    async fn default_secure_screen_mode_is_none() {
        assert!(AppConfig::default().secure_screen_mode.is_none());
    }

    /// `#[serde(other)]` sinks a value written by a newer build to `Unknown`
    /// instead of failing deserialization (which would wipe the whole config).
    /// The frontend resolves `Unknown` to the sensitive default. Tested at the
    /// serde layer directly so the assertion survives the split (which moves
    /// `secure_screen_mode` into the sealed behavior file).
    #[tokio::test]
    async fn secure_screen_mode_unknown_sinks_via_serde_other() {
        let json = r#"{"secure_screen_mode":"some-future-mode"}"#;
        let cfg: AppConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.secure_screen_mode, Some(SecureScreenMode::Unknown));
        // The sealed-behavior half carries the same serde sink.
        let behavior: BehaviorConfig = serde_json::from_str(json).unwrap();
        assert_eq!(behavior.secure_screen_mode, Some(SecureScreenMode::Unknown));
    }

    #[tokio::test]
    async fn secure_screen_mode_roundtrips_through_save() {
        let dir = tempdir().expect("tempdir");
        let store = store_with_desktop_store(dir.path()).await;
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
                store_with_desktop_store(dir.path())
                    .await
                    .get()
                    .secure_screen_mode,
                Some(mode),
                "{mode:?} round-trips",
            );
        }
    }

    #[tokio::test]
    async fn secure_screen_mode_omitted_on_disk_when_none() {
        // skip_serializing_if keeps the field out of app.json while None, so a
        // default config stays byte-identical. The legacy write path is used
        // here because the post-split write goes through the sealed behavior
        // slot (which on desktop is plaintext-passthrough JSON of the
        // BehaviorConfig shape — also omits the field, but the assertion text
        // would need to know the new shape).
        let dir = tempdir().expect("tempdir");
        let store = store_at(dir.path()).await;
        store
            .save_legacy_app_json(&LegacyAppConfig {
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
        let store = store_with_desktop_store(dir.path()).await;
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

    #[tokio::test]
    async fn serde_missing_key_schema_default_stays_at_one() {
        // The serde missing-key default stays at 1: a pre-split app.json that
        // omits the key must still run the registry (otherwise it would skip
        // straight to the target and silently lose the scope split + the
        // bool→mode conversion + the sealed-behavior split). A brand-new config
        // uses AppConfig::default / PrefConfig::default, tested below.
        assert_eq!(default_schema_version(), 1);
    }

    #[tokio::test]
    async fn default_config_starts_at_current_schema_target() {
        // A brand-new install skips the legacy no-op migrations by starting at
        // the registry's target. (Existing files keep their own schema_version;
        // only a missing key falls back to the serde default of 1.)
        assert_eq!(
            AppConfig::default().schema_version,
            crate::migrations::APP_CONFIG_SCHEMA_VERSION,
        );
        assert_eq!(
            PrefConfig::default().schema_version,
            crate::migrations::APP_CONFIG_SCHEMA_VERSION,
        );
    }

    /// The display half of a `LegacyAppConfig` lifts cleanly into a `PrefConfig`,
    /// preserving all display fields + `schema_version`. The deprecated
    /// `secure_screen`/`log_level` fields don't survive into `PrefConfig`
    /// (they live only in the V1–V3 snapshots, consumed by `m0003`/`m0004`).
    #[tokio::test]
    async fn pref_config_from_legacy_preserves_display_fields() {
        let app = LegacyAppConfig {
            secure_screen: false,
            secure_screen_mode: Some(SecureScreenMode::Off),
            locale: Some("zh-CN".to_string()),
            theme_mode: Some("dark".to_string()),
            log_level: Some("debug".to_string()),
            schema_version: 3,
            ..Default::default()
        };
        let pref = PrefConfig::from_legacy(&app);
        assert_eq!(pref.locale.as_deref(), Some("zh-CN"));
        assert_eq!(pref.theme_mode.as_deref(), Some("dark"));
        assert_eq!(pref.schema_version, 3);
    }

    /// The behavior half of a `LegacyAppConfig` lifts cleanly into a
    /// `BehaviorConfig`, preserving all six behavior fields. Pins the legacy
    /// lift round-trip.
    #[tokio::test]
    async fn behavior_config_from_legacy_preserves_behavior_fields() {
        let app = LegacyAppConfig {
            secure_screen_mode: Some(SecureScreenMode::Always),
            lock_mode: LockMode::Idle(300),
            view_clear_secs: Some(0),
            clipboard_clear_secs: Some(180),
            autosync: false,
            biometric_app_lock: true,
            ..Default::default()
        };
        let b = BehaviorConfig::from_legacy(&app);
        assert_eq!(b.secure_screen_mode, Some(SecureScreenMode::Always));
        assert_eq!(b.lock_mode, LockMode::Idle(300));
        assert_eq!(b.view_clear_secs, Some(0));
        assert_eq!(b.clipboard_clear_secs, Some(180));
        assert!(!b.autosync);
        assert!(b.biometric_app_lock);
    }

    /// `LegacyAppConfig::default` agrees with the serde defaults — `secure_screen`
    /// defaults ON (not the derived `bool` false), matching what a missing key
    /// would deserialize to. Without this, the legacy lift of a partially-
    /// populated file would silently downgrade screen-capture protection.
    #[tokio::test]
    async fn legacy_app_config_default_secure_screen_is_true() {
        assert!(
            LegacyAppConfig::default().secure_screen,
            "LegacyAppConfig::default must agree with the serde default (true)"
        );
    }

    #[tokio::test]
    async fn gate_idle_default_is_after_300() {
        // The unified default for both the IPC view and the sealed behavior
        // half — new installs get a 5-min idle timeout.
        assert_eq!(
            AppConfig::default().gate_idle,
            GateIdle::After(GATE_IDLE_DEFAULT_SECS)
        );
        assert_eq!(
            BehaviorConfig::default().gate_idle,
            GateIdle::After(GATE_IDLE_DEFAULT_SECS)
        );
    }

    #[tokio::test]
    async fn gate_idle_serde_round_trips() {
        for mode in [
            GateIdle::Off,
            GateIdle::After(600),
            GateIdle::After(GATE_IDLE_DEFAULT_SECS),
        ] {
            let json = serde_json::to_string(&mode).unwrap();
            assert_eq!(serde_json::from_str::<GateIdle>(&json).unwrap(), mode);
        }
        // The default is omitted from the behavior file (skip_serializing_if).
        assert!(
            !serde_json::to_string(&BehaviorConfig::default())
                .unwrap()
                .contains("gate_idle")
        );
    }

    #[tokio::test]
    async fn gate_idle_omitted_on_disk_when_default() {
        // skip_serializing_if keeps gate_idle out of the behavior file while at
        // the default, so a fresh config stays byte-identical.
        let dir = tempdir().expect("tempdir");
        let store = store_with_desktop_store(dir.path()).await;
        store
            .save_behavior(&BehaviorConfig::default())
            .await
            .unwrap();
        let on_disk = std::fs::read_to_string(dir.path().join(APP_CONFIG_FILE)).unwrap();
        assert!(
            !on_disk.contains("gate_idle"),
            "gate_idle must be absent at default; got: {on_disk}",
        );
    }

    #[tokio::test]
    async fn gate_idle_round_trips_off_and_after() {
        let dir = tempdir().expect("tempdir");
        for mode in [GateIdle::Off, GateIdle::After(900)] {
            let store = store_with_desktop_store(dir.path()).await;
            store.set_gate_idle(mode).await.unwrap();
            assert_eq!(
                store_with_desktop_store(dir.path()).await.get().gate_idle,
                mode,
                "{mode:?} round-trips",
            );
        }
    }

    #[tokio::test]
    async fn clamp_gate_idle_keeps_off_and_clamps_after() {
        assert_eq!(clamp_gate_idle(GateIdle::Off), GateIdle::Off);
        assert_eq!(
            clamp_gate_idle(GateIdle::After(60)),
            GateIdle::After(GATE_IDLE_SECS_MIN)
        );
        assert_eq!(clamp_gate_idle(GateIdle::After(300)), GateIdle::After(300));
        assert_eq!(
            clamp_gate_idle(GateIdle::After(9999)),
            GateIdle::After(GATE_IDLE_SECS_MAX)
        );
    }

    #[tokio::test]
    async fn behavior_config_from_legacy_defaults_gate_idle() {
        // gate_idle is a new field — from_legacy does not carry it from a legacy
        // config (which never had it); it defaults, and m0006 later pins existing
        // users to Off.
        let app = LegacyAppConfig::default();
        assert_eq!(
            BehaviorConfig::from_legacy(&app).gate_idle,
            GateIdle::default()
        );
    }

    /// A post-split `pref.json` is preferred over the legacy `app.json` lift.
    #[tokio::test]
    async fn pref_json_preferred_over_legacy_app_json_when_present() {
        let dir = tempdir().expect("tempdir");
        // Stale legacy file (would lift different values if used).
        std::fs::write(dir.path().join(APP_CONFIG_FILE), r#"{"locale":"en"}"#).unwrap();
        // pref.json wins.
        std::fs::write(
            dir.path().join(PREF_FILE),
            r#"{"locale":"zh-CN","schema_version":4}"#,
        )
        .unwrap();
        let cfg = store_at(dir.path()).await.get();
        assert_eq!(
            cfg.locale.as_deref(),
            Some("zh-CN"),
            "pref.json must win over the legacy lift"
        );
        assert_eq!(cfg.schema_version, 4);
    }
}
