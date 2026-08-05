// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Migration `0003_secure_screen_mode`.
//!
//! Converts the deprecated boolean `secure_screen` toggle (carried by the
//! schema-2 [`super::m0002_config_scope_split::AppConfigV2`]) into the three-state
//! `secure_screen_mode`: `false → Off`, `true`/missing → `None` (the `Sensitive`
//! default, so a default user's on-disk shape stays byte-identical to a fresh
//! install). Produces the schema-3 shape ([`AppConfigV3`]), which drops the bool
//! (consumed here).
//!
//! The source is read raw off disk as
//! [`super::m0002_config_scope_split::AppConfigV2`]. An earlier revision of this
//! migration warned against a raw-at-load snapshot ("a crash/save-failure window
//! could silently flip `Off → Sensitive`"); that warning is moot under the
//! versioned-snapshot model — the raw read is idempotent, and a write failure
//! leaves `schema_version` at 2 so the next run re-reads the same V2 and re-runs
//! this step. A pinned mode is never flipped: the preserve-already-pinned branch
//! below runs again identically.
//!
//! Idempotent (the engine gates on `schema_version`).

use rustpass::{Error, LockMode};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::app_config::{SecureScreenMode, default_autosync_true, is_autosync_default, is_false};
use crate::migrations::MigrationOutcome;
use crate::migrations::m0002_config_scope_split::AppConfigV2;

/// Serde default for [`AppConfigV3::schema_version`] — `3`, the version this
/// migration produces.
fn default_schema_v3() -> u32 {
    3
}

/// The on-disk `app.json` shape at schema 3: the `secure_screen` bool is gone
/// (consumed by this migration), replaced by `secure_screen_mode`. Still
/// carries the deprecated `log_level` (consumed by `m0004`). `m0004` reads this
/// raw as its source shape.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct AppConfigV3 {
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) verbose_until: Option<u64>,
    #[serde(default = "default_schema_v3")]
    pub(crate) schema_version: u32,
}

/// Manual `Default` so `autosync` matches its serde default (`true`), not the
/// derived `bool` default (`false`). m0004 falls back to
/// `AppConfigV3::default()` when the on-disk V3 is unparseable (a wrong-type
/// field that `peek_schema_version` still parses); a derived `false` would
/// disagree with the serde default and silently flip autosync off.
impl Default for AppConfigV3 {
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
            log_level: None,
            verbose_until: None,
            schema_version: default_schema_v3(),
        }
    }
}

/// Convert the deprecated `secure_screen` bool into `secure_screen_mode` and
/// bump `schema_version` to 3. See the module docs for the mapping and the
/// rationale for reading the V2 source raw instead of off the cache.
pub(crate) async fn apply(state: &AppState, version: u32) -> Result<MigrationOutcome, Error> {
    // Read the schema-2 app.json. The engine only runs us when peek saw a
    // schema < 3 (so the file exists); a full V2 parse failure (a wrong-type
    // field that peek still parses) is rare — fall back to the V2 defaults so
    // the chain heals the file to target instead of warn-looping every launch.
    let v2: AppConfigV2 = state
        .app_config
        .read_app_json_as()
        .await
        .unwrap_or_else(|e| {
            log::warn!("0003_secure_screen_mode: app.json unparseable ({e}); using V2 defaults");
            AppConfigV2::default()
        });
    // Only set the mode when it is not already pinned (a partially-migrated
    // file re-running this step keeps any explicit value). false → Off;
    // true/missing → None — None is the Sensitive default, so a default user's
    // app.json stays byte-identical.
    let secure_screen_mode = if v2.secure_screen_mode.is_some() {
        v2.secure_screen_mode
    } else {
        (!v2.secure_screen).then_some(SecureScreenMode::Off)
    };
    let v3 = AppConfigV3 {
        secure_screen_mode,
        locale: v2.locale,
        theme_mode: v2.theme_mode,
        lock_mode: v2.lock_mode,
        view_clear_secs: v2.view_clear_secs,
        clipboard_clear_secs: v2.clipboard_clear_secs,
        autosync: v2.autosync,
        biometric_app_lock: v2.biometric_app_lock,
        log_level: v2.log_level,
        verbose_until: None,
        schema_version: version,
    };
    state.app_config.write_app_json_raw(&v3).await?;
    Ok(MigrationOutcome::Done)
}
