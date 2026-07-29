// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! App-shell configuration that must persist before any repo is set up, and
//! survive a repository re-setup.
//!
//! # The three persistence tiers
//!
//! gpm persists state across three tiers; this module owns the third:
//!
//! 1. **Git** — the cloned gopass repository of age-encrypted secrets, version-
//!    controlled and synced via `git pull`/`push`. The only tier that leaves the
//!    device. (The on-disk clone lives under the path `repo.json` points at.)
//! 2. **Sealed files** — `repo.json` (repo-scoped config), `identity`, and the
//!    post-split behavior slot in `app.json`, sealed at rest with AEAD on
//!    Android, plaintext on desktop. Owned by `rustpass`. See
//!    [`rustpass::config::Config`].
//! 3. **Plaintext files** — **`pref.json` (this module)**, always plaintext.
//!
//! # The display/behavior split
//!
//! The single plaintext `app.json` is split into two files:
//! - **`pref.json`** (plaintext, this module) — display prefs that must render
//!   before unlock: `locale`, `theme_mode`, `verbose_until`, and `schema_version`.
//! - **`app.json`** (sealed via `Store::save_app_behavior`) — behavior prefs
//!   that are confidential security choices: `lock_mode`, the view/clipboard
//!   clear timers, `autosync`, `biometric_app_lock`, `secure_screen_mode`. On
//!   Android these are AEAD-sealed under the master key (unreadable until
//!   unlock); on desktop the seal is passthrough plaintext.
//!
//! `pref.json` is plaintext on disk, and this is forced, not a shortcut: the
//! `locale` must be readable before unlock (first-paint injection + the app-lock
//! biometric screen), and sealing it would make it unreadable at setup
//! when app-lock is on. None of these prefs are confidential, and the local
//! write attacker is out of scope per the threat model, so plaintext is
//! consistent. (The `WebView`'s `localStorage` is explicitly not a tier — it
//! may be cleared by the system, so it is never authoritative for settings.)
//!
//! `m0005` owns the split: it reads the legacy plaintext single-file `app.json`
//! as the schema-4 snapshot (`AppConfigV4`, defined in `m0004`), writes the
//! display half to `pref.json`, then seals the behavior half via the Store. The
//! schema version (tracked in `pref.json` post-split) advances only after the
//! sealed write succeeds, so a Pending (app-lock) resume re-enters cleanly.
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
//! `pref.json` and the sealed `app.json` both intentionally survive
//! `reset_config` (which wipes the repo dir, `identity`, `repo.json`, and the
//! `app_id_pass` slot): these are device-level preferences, not repo data, so
//! re-setting up the repo does not reset the user's language, timers, autosync,
//! or app-lock choice.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use rustpass::config::save_atomic;
use rustpass::{Error, ErrorCode, LockMode, Store, clamp_lock_mode, normalize_clear_secs};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::AppState;
use crate::verbose::{arm_verbose_timer, disarm_verbose_timer};

/// File name of the plaintext display-prefs file (post-split).
const PREF_FILE: &str = "pref.json";

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

/// App-level (non-repo) preferences — the **merged IPC view** of the plaintext
/// display prefs ([`PrefConfig`]) and the sealed behavior prefs
/// ([`BehaviorConfig`]). Constructed on demand by [`AppConfigStore::get`];
/// never persisted as a single shape post-split. The legacy single-file shape
/// (still carried by main's schema-1–4 `app.json` files) is preserved as
/// [`LegacyAppConfig`] for the pre-split lift.
///
/// Field docs intentionally describe semantics rather than storage location
/// (which the split redistribute); see [`PrefConfig`] and [`BehaviorConfig`]
/// for which side of the split each field lives on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AppConfig {
    /// Three-state screen-capture protection (sealed app.json). `None` (the
    /// default) ⇒ `Sensitive` (the frontend resolves `None`/`Unknown` to
    /// `Sensitive`): sensitive routes + nav transitions + the unlock overlay
    /// block capture, the entry list / history stay capturable. `Off` ⇒ no
    /// screen is ever secured (the user explicitly allowed capture, including
    /// the unlock overlay). `Always` ⇒ every screen is secured at all times.
    /// `skip_serializing_if` keeps the field out of `app.json` while `None`, so
    /// a default config stays byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) secure_screen_mode: Option<SecureScreenMode>,
    /// Display-language override (pref.json). `None` (the default) means "track
    /// the system language" — the backend resolves the system locale at boot.
    /// `Some("en")` / `Some("zh-CN")` pins the locale explicitly.
    /// `skip_serializing_if` keeps existing files (which predate this field)
    /// byte-identical on round-trip, so adding the field is non-breaking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) locale: Option<String>,
    /// Color-scheme (light/dark) override (pref.json). `None` (the default)
    /// means "track the system preference" — the frontend's
    /// `prefers-color-scheme` CSS media query governs, zero-JS and zero-flash.
    /// `Some("light")` / `Some("dark")` pins it via a `<html data-theme>`
    /// attribute the frontend sets after reading this. Plaintext here (not
    /// sealed) for the same reason as `locale`: it must render before unlock
    /// and survive `reset_config`. `skip_serializing_if` keeps existing files
    /// byte-identical on round-trip, so adding the field is non-breaking
    /// (mirrors `locale`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) theme_mode: Option<String>,
    /// How the app auto-locks the identity cache (sealed app.json). Skipped
    /// from serialization when default (`Immediate`), so an uncustomized config
    /// is byte-identical to one written before this field moved here.
    #[serde(default, skip_serializing_if = "LockMode::is_default")]
    pub(crate) lock_mode: LockMode,
    /// Seconds a revealed password stays in the DOM before auto-clear (sealed
    /// app.json). `None` ⇒ [`DEFAULT_VIEW_CLEAR_SECS`]; `Some(0)` ⇒ never
    /// auto-clear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) view_clear_secs: Option<u64>,
    /// Seconds the clipboard holds a copied password before auto-clear (sealed
    /// app.json). `None` ⇒ [`DEFAULT_CLIPBOARD_CLEAR_SECS`]; `Some(0)` ⇒ never
    /// auto-clear.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) clipboard_clear_secs: Option<u64>,
    /// Whether each save wraps in a pull→write→push (gopass-style per-command
    /// sync) (sealed app.json). Default `true`; omitted from serialization
    /// while `true`.
    #[serde(
        default = "default_autosync_true",
        skip_serializing_if = "is_autosync_default"
    )]
    pub(crate) autosync: bool,
    /// Persisted intent for the app-launch biometric gate (sealed app.json).
    /// **Write-only** — the Settings toggle and the runtime gate read the
    /// Keystore probe via `get_app_lock_state`, not this flag; it exists only
    /// as a persisted record mirroring the old `RepoConfig` field. Skipped when
    /// `false`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) biometric_app_lock: bool,
    /// App-launch-gate in-app idle timeout (sealed app.json). Defaults to
    /// `After(300)` (5 min) for new installs; `m0006` sets existing configs to
    /// `Off`. `skip_serializing_if` omits the default so a fresh config stays
    /// byte-identical.
    #[serde(default, skip_serializing_if = "GateIdle::is_default")]
    pub(crate) gate_idle: GateIdle,
    /// Persisted-schema version for one-shot migrations (pref.json). `1` is the
    /// pre-split shape; the `migrations` registry bumps it as each step runs
    /// (target: `migrations::APP_CONFIG_SCHEMA_VERSION`).
    #[serde(default = "default_schema_version")]
    pub(crate) schema_version: u32,
    /// Verbose-logging deadline as Unix seconds, or `None` (the Info default)
    /// (pref.json). While set and not yet past, the runtime log gate is Debug;
    /// once past (or `None`) it is Info. Persisted in plaintext (same rationale
    /// as `locale`: readable before unlock, survives `reset_config`,
    /// non-confidential) so a launch made with verbose on records startup at
    /// Debug. See RFC 0055. Omitted while `None` so a default config stays
    /// byte-identical on round-trip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) verbose_until: Option<u64>,
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
        }
    }
}

/// Plaintext display preferences — the `pref.json` half of the split. Read
/// pre-unlock (locale/theme/log must render before the identity is decrypted),
/// so this file stays plaintext.
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
        }
    }
}

/// Behavior preferences — the sealed `app.json` half of the split. Sealed
/// because behavior is a confidential security choice (the user's lock timeout,
/// autosync, biometric, screen-capture mode). On Android these are AEAD-sealed
/// under the master key (unreadable until unlock); on desktop the seal is
/// passthrough plaintext. Same serde attrs as the equivalent [`AppConfig`]
/// fields so the post-split file shape mirrors the legacy single-file shape
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

/// The locale to bake into the `WebView` initialization script.
///
/// This runs at Tauri `Builder` time, before the `App` exists — so on Android
/// the config directory (and thus `pref.json`/`app.json`) is not yet readable
/// (it is only resolvable through the running app's mobile-plugin IPC). The
/// system locale is readable this early, though (`sys_locale` reads it via
/// libc, no app required, on every platform), so the inject carries the "track
/// system" resolution. This is exactly correct for users who haven't pinned a
/// language (the default, and the first-launch case), and the boot
/// `resolved_locale` IPC corrects it within one frame for users who have.
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

/// Read `app.json` (the pre-split single-file shape) from `config_dir` and
/// parse it as the [`LegacyAppConfig`] shape. Returns `None` if the file is
/// missing or unparseable — used by [`AppConfigStore::new`] (the legacy lift),
/// the engine's end-of-chain reload, and `m0005`'s half-migrated recovery. The
/// byte-oriented sealed behavior slot (post-split) does NOT parse as
/// [`LegacyAppConfig`] cleanly (carries only the behavior subset), so callers
/// dispatching on the file shape should check [`rustpass::seal::is_envelope`]
/// first to tell a sealed slot apart from a plaintext legacy file.
fn load_legacy_app_json_at(path: &Path) -> Option<LegacyAppConfig> {
    let s = std::fs::read_to_string(path).ok()?;
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

/// Persistent app-shell config, owned by [`AppState`]. Two-phase: constructed
/// without a `Store` (so the migration registry can run before the Store is
/// built if needed), then [`set_store`](Self::set_store) binds the Store so
/// sealed behavior writes/reads can flow. Plaintext `pref.json` is read once
/// synchronously at construction (lifting the legacy `app.json` display fields
/// when `pref.json` is absent); sealed `app.json` is loaded post-unlock via
/// [`reload_behavior`](Self::reload_behavior). The in-memory caches are
/// authoritative thereafter; the [`Mutex`] guards are never held across an
/// `.await`.
#[derive(Debug)]
pub(crate) struct AppConfigStore {
    pref_path: PathBuf,
    app_json_path: PathBuf,
    pref: Mutex<PrefConfig>,
    behavior: Mutex<BehaviorConfig>,
    /// Late-bound Store ref so setter signatures stay stable (no `&Store`
    /// parameter) and so callers in `config.rs`/`applock.rs` don't change. Set
    /// once via [`set_store`](Self::set_store) right after the Store is built.
    store: OnceLock<Arc<Store>>,
    /// Staged text for the verbose-revert OS notification (posted by the
    /// deadline timer). Memory-only; `None` until verbose is enabled.
    revert_notify: Mutex<Option<VerboseNotifyText>>,
}

impl AppConfigStore {
    /// Load the display prefs from `config_dir/pref.json`, falling back to the
    /// legacy lift from `config_dir/app.json` when `pref.json` is absent (the
    /// pre-split case), and finally to defaults. The behavior cache starts at
    /// default — sealed behavior is loaded post-unlock via
    /// [`reload_behavior`](Self::reload_behavior).
    ///
    /// Resilience: a missing file (fresh install) is normal — silent default.
    /// A present-but-unreadable or corrupt file would silently revert
    /// `locale`/`theme_mode`/`verbose_until` to defaults; warn so the revert
    /// leaves a trace (the file is plaintext, so the warn carries no secret).
    #[must_use]
    pub(crate) fn new(config_dir: &Path) -> Self {
        let pref_path = config_dir.join(PREF_FILE);
        let app_json_path = config_dir.join(APP_CONFIG_FILE);
        // Prefer pref.json (post-split shape); fall back to the legacy lift from
        // app.json so a pre-split file's prefs survive the upgrade; finally
        // default. The legacy lift populates BOTH caches: display into pref, and
        // behavior into the behavior cache — otherwise a pre-split writer
        // (m0002/m0003) that does `get()`→`save_legacy_app_json` would overwrite
        // the seeded behavior with defaults (the behavior cache starts empty
        // post-split and is loaded post-unlock via `reload_behavior`, but the
        // legacy file still carries behavior pre-split). schema_version is
        // preserved (the registry bumps it as migrations run).
        let (pref, behavior) = if pref_path.exists() {
            let pref = match std::fs::read_to_string(&pref_path) {
                Ok(s) => serde_json::from_str::<PrefConfig>(&s).unwrap_or_else(|e| {
                    log::warn!("app-config: corrupt pref.json, using defaults: {e}");
                    PrefConfig::default()
                }),
                Err(e) => {
                    log::warn!("app-config: pref.json unreadable, using defaults: {e}");
                    PrefConfig::default()
                }
            };
            (pref, BehaviorConfig::default())
        } else if let Some(legacy) = load_legacy_app_json_at(&app_json_path) {
            (
                PrefConfig::from_legacy(&legacy),
                BehaviorConfig::from_legacy(&legacy),
            )
        } else {
            (PrefConfig::default(), BehaviorConfig::default())
        };
        Self {
            pref_path,
            app_json_path,
            pref: Mutex::new(pref),
            behavior: Mutex::new(behavior),
            store: OnceLock::new(),
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
    /// complete, so existence is a reliable split signal.
    pub(crate) fn pref_json_exists(&self) -> bool {
        self.pref_path.exists()
    }

    /// Read `app.json` (the pre-split single-file shape) as raw text and
    /// deserialize into `T`. Plaintext analog of
    /// [`rustpass::Store::load_repo_config_as`] minus the unseal step. Used by
    /// each migration to read its own source-version snapshot. Sync — the read
    /// is tiny and [`AppConfigStore::new`] already reads synchronously.
    pub(crate) fn read_app_json_as<T: serde::de::DeserializeOwned>(&self) -> Result<T, Error> {
        let s = std::fs::read_to_string(&self.app_json_path)?;
        Ok(serde_json::from_str(&s)?)
    }

    /// Minimal raw read of the persisted schema version, for the migration
    /// engine's gate. Dual-file: post-split the schema lives in `pref.json`,
    /// pre-split it lives in `app.json`. Returns `None` when both are missing or
    /// unparseable; the engine treats `None` as "skip all migrations" (a
    /// missing/corrupt state is a fresh install or post-reset, not a schema to
    /// migrate).
    pub(crate) fn peek_schema_version(&self) -> Option<u32> {
        // Post-split (pref.json exists) — schema_version lives there.
        if self.pref_path.exists()
            && let Ok(s) = std::fs::read_to_string(&self.pref_path)
            && let Ok(p) = serde_json::from_str::<SchemaVersionPeek>(&s)
        {
            return Some(p.schema_version);
        }
        // Pre-split OR pref.json corrupt/missing — fall back to app.json.
        let s = std::fs::read_to_string(&self.app_json_path).ok()?;
        serde_json::from_str::<SchemaVersionPeek>(&s)
            .ok()
            .map(|p| p.schema_version)
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

    /// Re-read `pref.json` into the pref cache and re-seal-load the behavior
    /// slot, after the migration chain has written fresh files. Called only by
    /// the engine at the end of a COMPLETED chain (`run_app_migrations`), where
    /// `pref.json` always exists at the target schema (m0005 wrote it and
    /// bumped it to 5 on its `Done` path). [`reload_behavior`](Self::reload_behavior)
    /// is the load-bearing piece for m0005's envelope-recovery case, where
    /// `new()` left behavior at default because `pref.json` already existed.
    ///
    /// The half-migrated behavior load (m0005 wrote `pref.json` at schema 4 but
    /// deferred `Pending` before sealing) does NOT route through here — the
    /// engine returns on `Pending` before calling this. That case is carried by
    /// [`AppConfigStore::new`]'s legacy lift (pref.json absent at construction)
    /// plus `init_state`'s standalone `reload_behavior()`, which parses the
    /// still-plaintext single-file `app.json` as `BehaviorConfig` via field
    /// overlap. See `reload_behavior_loads_half_migrated_plaintext_app_json`.
    ///
    /// Errors are propagated (the chain wrote a valid file; a reload failure is
    /// worth surfacing). The engine log+warns on a reload error rather than
    /// propagating further.
    pub(crate) async fn reload(&self) -> Result<(), Error> {
        // Pref refresh (defensive — m0005's save_pref + schema bump already
        // swapped the cache; re-reading keeps this robust if a future migration
        // path ever skips that swap). pref.json is always present here.
        if self.pref_path.exists() {
            let s = std::fs::read_to_string(&self.pref_path)?;
            let pref: PrefConfig = serde_json::from_str(&s)?;
            *self.pref.lock().expect("pref lock poisoned") = pref;
        }
        // Behavior slot — reload_behavior soft-fails to the cache on
        // NoIdentity/SealKeyUnavailable/parse errors (mirrors `new()`).
        self.reload_behavior().await
    }

    /// Snapshot the plaintext pref cache.
    pub(crate) fn get_pref(&self) -> PrefConfig {
        self.pref.lock().expect("pref lock poisoned").clone()
    }

    /// Snapshot the sealed behavior cache. The cache starts at default at
    /// construction; populate it via
    /// [`reload_behavior`](Self::reload_behavior) post-unlock.
    pub(crate) fn get_behavior(&self) -> BehaviorConfig {
        self.behavior
            .lock()
            .expect("behavior lock poisoned")
            .clone()
    }

    /// Merge the pref + behavior caches into an [`AppConfig`] (the IPC view).
    /// This is what `get_app_config`, `apply_security_caches`, and tests
    /// consume — it stays the superset of every field that previously lived in
    /// the single-file `app.json`, so callers that don't care about the split
    /// see no change.
    pub(crate) fn get(&self) -> AppConfig {
        let pref = self.pref.lock().expect("pref lock poisoned");
        let behavior = self.behavior.lock().expect("behavior lock poisoned");
        AppConfig {
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
        }
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
    /// to persist the revert. Reads the plaintext pref cache (`verbose_until`
    /// lives in `pref.json`), so this is safe to call pre-unlock.
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
        self.update_pref(|p| p.verbose_until = on.then(|| now_unix() + VERBOSE_WINDOW_SECS))
            .await
    }

    /// Persist-clear an expired verbose deadline (keeps `pref.json` tidy once the
    /// window has passed). Best-effort at startup: the level is already lazy-Info
    /// via [`Self::effective_log_filter`], so a failure here is non-fatal — the
    /// next launch retries. Re-checks expiry INSIDE the closure so a `set_verbose`
    /// landing between the read and the swap is not clobbered.
    pub(crate) async fn clear_expired_verbose(&self) -> Result<(), Error> {
        if self.get_pref().verbose_until.is_none() {
            return Ok(());
        }
        self.update_pref(|p| {
            if p.verbose_until.is_some_and(|d| d <= now_unix()) {
                p.verbose_until = None;
            }
        })
        .await?;
        Ok(())
    }

    /// Persist `cfg` to `pref.json` atomically (via `rustpass::config::save_atomic`
    /// — temp + rename, DRY) and update the pref cache.
    ///
    /// The `Mutex` is held only for the final cache swap — never across the
    /// `tokio::fs` `.await` points (the write/rename complete before the guard
    /// is taken), so there is no await-held-lock deadlock risk.
    pub(crate) async fn save_pref(&self, cfg: &PrefConfig) -> Result<(), Error> {
        let json = serde_json::to_string_pretty(cfg)?;
        save_atomic(&self.pref_path, json.as_bytes()).await?;
        *self.pref.lock().expect("pref lock poisoned") = cfg.clone();
        Ok(())
    }

    /// Serialize `cfg` to bytes and seal them into `app.json` via the bound
    /// Store's `save_app_behavior`. The Seal itself gates: passthrough on
    /// desktop (key `None`), `SealKeyUnavailable` if ever-keyed-then-wiped (the
    /// app-launch lock cold-start path). No separate `app_locked` reject — it
    /// would wrongly reject desktop.
    ///
    /// Updates the behavior cache so a subsequent [`get`](Self::get) reflects
    /// the new value without a round-trip through disk.
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
        *self.behavior.lock().expect("behavior lock poisoned") = cfg.clone();
        Ok(())
    }

    /// Read + unseal `app.json` and refresh the behavior cache. Soft-fails to
    /// defaults on `NoIdentity` (missing slot, pre-unlock) and
    /// `SealKeyUnavailable` (master key not yet injected) — both are normal
    /// pre-unlock states, not errors. Mirrors `new()`'s resilience on
    /// parse/IO errors (warn + leave the cache at the last-read value).
    pub(crate) async fn reload_behavior(&self) -> Result<(), Error> {
        let Some(store) = self.store.get() else {
            // No Store bound — nothing to load. Leave the cache at defaults.
            return Ok(());
        };
        match store.load_app_behavior().await {
            Ok(bytes) => match serde_json::from_slice::<BehaviorConfig>(&bytes) {
                Ok(cfg) => {
                    *self.behavior.lock().expect("behavior lock poisoned") = cfg;
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

    /// Persist `cfg` as a plaintext legacy single-file `app.json` (all fields)
    /// via `save_atomic` and update BOTH caches to mirror the write. Test-only —
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
        // Keep both caches in sync with the legacy write so a subsequent get()
        // reflects the new value without a round-trip through disk.
        *self.pref.lock().expect("pref lock poisoned") = PrefConfig::from_legacy(cfg);
        *self.behavior.lock().expect("behavior lock poisoned") = BehaviorConfig::from_legacy(cfg);
        Ok(())
    }

    /// Get → mutate → save → return the merged config. Shared shape for the
    /// pref setters (atomic write + cache swap, never holding the mutex across
    /// an `.await`).
    async fn update_pref<F: FnOnce(&mut PrefConfig)>(&self, f: F) -> Result<AppConfig, Error> {
        let mut pref = self.get_pref();
        f(&mut pref);
        self.save_pref(&pref).await?;
        Ok(self.get())
    }

    /// Same shape as [`update_pref`](Self::update_pref), for the behavior
    /// setters. Requires the Store to be bound (the sealed write flows through
    /// it).
    async fn update_behavior<F: FnOnce(&mut BehaviorConfig)>(
        &self,
        f: F,
    ) -> Result<AppConfig, Error> {
        let mut behavior = self.get_behavior();
        f(&mut behavior);
        self.save_behavior(&behavior).await?;
        Ok(self.get())
    }

    /// Set the auto-lock mode (sealed behavior). `Idle(n)` is clamped first.
    pub(crate) async fn set_lock_mode(&self, mode: LockMode) -> Result<AppConfig, Error> {
        self.update_behavior(|b| b.lock_mode = clamp_lock_mode(mode))
            .await
    }

    /// Set the password-view auto-clear override (sealed behavior). `None` ⇒
    /// default, `Some(0)` ⇒ never, else clamped to the allowed range.
    pub(crate) async fn set_view_clear_secs(&self, secs: Option<u64>) -> Result<AppConfig, Error> {
        self.update_behavior(|b| b.view_clear_secs = normalize_clear_secs(secs))
            .await
    }

    /// Set the clipboard auto-clear override (sealed behavior, same rule as
    /// view-clear).
    pub(crate) async fn set_clipboard_clear_secs(
        &self,
        secs: Option<u64>,
    ) -> Result<AppConfig, Error> {
        self.update_behavior(|b| b.clipboard_clear_secs = normalize_clear_secs(secs))
            .await
    }

    /// Set the per-save autosync flag (sealed behavior).
    pub(crate) async fn set_autosync(&self, enabled: bool) -> Result<AppConfig, Error> {
        self.update_behavior(|b| b.autosync = enabled).await
    }

    /// Set the persisted app-launch biometric-gate intent flag (sealed
    /// behavior; write-only mirror of the Keystore-probed runtime state).
    pub(crate) async fn set_biometric_app_lock(&self, enabled: bool) -> Result<AppConfig, Error> {
        self.update_behavior(|b| b.biometric_app_lock = enabled)
            .await
    }

    /// Set the app-launch-gate in-app idle timeout (sealed behavior). `After(n)`
    /// is clamped to the preset range first. The Tauri `set_gate_idle` command
    /// applies the new value to the live backend timer (R057); this store method
    /// only persists + returns the updated config.
    pub(crate) async fn set_gate_idle(&self, mode: GateIdle) -> Result<AppConfig, Error> {
        self.update_behavior(|b| b.gate_idle = clamp_gate_idle(mode))
            .await
    }

    /// Set the persisted color-scheme override (pref.json) (`None` ⇒ track
    /// system). `Some` must be one of [`SUPPORTED_THEME_MODES`]; a bad value
    /// returns `ConfigError`. The frontend applies the runtime effect (the
    /// `data-theme` attribute) on receipt, so this stays a pure persistence
    /// step mirroring `set_locale`.
    pub(crate) async fn set_theme_mode(&self, mode: Option<String>) -> Result<AppConfig, Error> {
        validate_theme_mode(mode.as_deref())?;
        self.update_pref(|p| p.theme_mode = mode).await
    }

    /// Set the display-language preference (pref.json) (`null` clears the
    /// override — track system; `"en"` / `"zh-CN"` pin it). Mirrors
    /// `set_theme_mode`. The frontend re-applies the locale on receipt.
    pub(crate) async fn set_locale(&self, locale: Option<String>) -> Result<AppConfig, Error> {
        validate_locale(locale.as_deref())?;
        self.update_pref(|p| p.locale = locale).await
    }

    /// Set the persisted three-state screen-capture mode (sealed behavior).
    /// Rejects [`SecureScreenMode::Unknown`] (a deserialization sink, not a
    /// settable value). The frontend re-applies the route's secure state on
    /// receipt, so this stays a pure persistence step mirroring `set_theme_mode`.
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
        self.update_behavior(|b| b.secure_screen_mode = Some(mode))
            .await
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

/// The authoritative locale the app should render in. The frontend uses this at
/// boot to reconcile against the best-effort value baked into the `WebView` init
/// script (which can only carry the system locale, not a pinned preference).
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
    app: tauri::AppHandle,
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

    fn store_at(dir: &Path) -> AppConfigStore {
        AppConfigStore::new(dir)
    }

    /// Bind a desktop-passthrough Store (`master_key = None`) so the sealed
    /// behavior setters/readers can flow. The seal is plaintext-passthrough in
    /// this mode, so behavior round-trips through `app.json` as plaintext JSON.
    async fn store_with_desktop_store(dir: &Path) -> AppConfigStore {
        let s = AppConfigStore::new(dir);
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
            store_at(dir.path()).get().secure_screen_mode.is_none(),
            "missing app.json must fall back to the default, not panic"
        );
    }

    #[tokio::test]
    async fn corrupt_file_defaults_sensitive_mode() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join(APP_CONFIG_FILE), "{not json").unwrap();
        assert!(
            store_at(dir.path()).get().secure_screen_mode.is_none(),
            "corrupt app.json must fall back to the default, not panic"
        );
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
            .save_legacy_app_json(&LegacyAppConfig {
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
        // skip_serializing_if keeps the field out of the file when it is None,
        // so existing files stay byte-identical and don't carry a null.
        let dir = tempdir().expect("tempdir");
        let store = store_at(dir.path());
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

    #[test]
    fn existing_app_json_without_locale_loads() {
        // An app.json written before the locale field existed must still parse,
        // with locale defaulting to None (backward compatibility).
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join(APP_CONFIG_FILE), "{}").unwrap();
        assert!(store_at(dir.path()).get().locale.is_none());
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
            .save_legacy_app_json(&LegacyAppConfig {
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

    #[test]
    fn existing_app_json_without_theme_mode_loads() {
        // An app.json written before theme_mode existed must still parse, with
        // theme_mode defaulting to None (backward compatibility — adding the
        // optional field is non-breaking, like locale).
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join(APP_CONFIG_FILE), "{}").unwrap();
        assert!(store_at(dir.path()).get().theme_mode.is_none());
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
    fn app_config_store_new_missing_file_uses_defaults() {
        let dir = tempdir().expect("tempdir");
        let store = AppConfigStore::new(dir.path());
        assert_eq!(
            store.get().schema_version,
            AppConfig::default().schema_version,
            "missing app.json must fall back to the default (current schema target)"
        );
    }

    #[test]
    fn app_config_store_new_corrupt_json_uses_defaults() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join(APP_CONFIG_FILE), "{not valid json").unwrap();
        let store = AppConfigStore::new(dir.path());
        assert_eq!(
            store.get().schema_version,
            AppConfig::default().schema_version,
            "corrupt app.json must fall back to the default, not panic"
        );
    }

    #[test]
    fn app_config_store_new_valid_file_loads_value() {
        let dir = tempdir().expect("tempdir");
        // A non-default value round-trips: secure_screen_mode "off" (default is
        // None / Sensitive).
        std::fs::write(
            dir.path().join(APP_CONFIG_FILE),
            serde_json::json!({ "secure_screen_mode": "off" }).to_string(),
        )
        .unwrap();
        let store = AppConfigStore::new(dir.path());
        assert_eq!(
            store.get().secure_screen_mode,
            Some(SecureScreenMode::Off),
            "a valid file's secure_screen_mode must load (not revert to default)"
        );
    }

    #[tokio::test]
    async fn verbose_until_roundtrips_through_pref() {
        let dir = tempdir().expect("tempdir");
        let store = store_at(dir.path());
        let pinned = now_unix() + 42;
        store
            .save_pref(&PrefConfig {
                verbose_until: Some(pinned),
                ..PrefConfig::default()
            })
            .await
            .unwrap();
        let reloaded = store_at(dir.path()).get_pref();
        assert_eq!(reloaded.verbose_until, Some(pinned));
    }

    #[tokio::test]
    async fn verbose_until_omitted_on_disk_when_none() {
        // skip_serializing_if keeps verbose_until out of pref.json while None,
        // so a default config stays byte-identical.
        let dir = tempdir().expect("tempdir");
        let store = store_at(dir.path());
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
        let store = store_at(dir.path());
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
        let store = store_at(dir.path());
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
        let store = store_at(dir.path());
        store.set_verbose(true).await.unwrap();
        let live = store.get_pref().verbose_until;
        store.clear_expired_verbose().await.unwrap();
        assert_eq!(
            store.get_pref().verbose_until,
            live,
            "a live verbose window is not cleared"
        );
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
        let store = store_at(dir.path());
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
    /// instead of failing deserialization (which would wipe the whole config).
    /// The frontend resolves `Unknown` to the sensitive default. Tested at the
    /// serde layer directly so the assertion survives the split (which moves
    /// `secure_screen_mode` into the sealed behavior file).
    #[test]
    fn secure_screen_mode_unknown_sinks_via_serde_other() {
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
        let store = store_at(dir.path());
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

    #[test]
    fn serde_missing_key_schema_default_stays_at_one() {
        // The serde missing-key default stays at 1: a pre-split app.json that
        // omits the key must still run the registry (otherwise it would skip
        // straight to the target and silently lose the scope split + the
        // bool→mode conversion + the sealed-behavior split). A brand-new config
        // uses AppConfig::default / PrefConfig::default, tested below.
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
        assert_eq!(
            PrefConfig::default().schema_version,
            crate::migrations::APP_CONFIG_SCHEMA_VERSION,
        );
    }

    /// The display half of a `LegacyAppConfig` lifts cleanly into a `PrefConfig`,
    /// preserving all display fields + `schema_version`. The deprecated
    /// `secure_screen`/`log_level` fields don't survive into `PrefConfig`
    /// (they live only in the V1–V3 snapshots, consumed by `m0003`/`m0004`).
    #[test]
    fn pref_config_from_legacy_preserves_display_fields() {
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
    #[test]
    fn behavior_config_from_legacy_preserves_behavior_fields() {
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
    #[test]
    fn legacy_app_config_default_secure_screen_is_true() {
        assert!(
            LegacyAppConfig::default().secure_screen,
            "LegacyAppConfig::default must agree with the serde default (true)"
        );
    }

    #[test]
    fn gate_idle_default_is_after_300() {
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

    #[test]
    fn gate_idle_serde_round_trips() {
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

    #[test]
    fn clamp_gate_idle_keeps_off_and_clamps_after() {
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

    #[test]
    fn behavior_config_from_legacy_defaults_gate_idle() {
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
        let cfg = store_at(dir.path()).get();
        assert_eq!(
            cfg.locale.as_deref(),
            Some("zh-CN"),
            "pref.json must win over the legacy lift"
        );
        assert_eq!(cfg.schema_version, 4);
    }
}
