// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Migration `0004_verbose_from_debug`.
//!
//! Carries a previously pinned `"debug"` log level (carried by the schema-3
//! [`super::m0003_secure_screen_mode::AppConfigV3`]) into the new verbose flag:
//! `log_level == "debug"` ⇒ `verbose_until = now + VERBOSE_WINDOW_SECS` (so the
//! upgrade is non-surprising — the user keeps Debug, then it expires under the
//! same time-box as any other verbose session); every other `log_level`
//! (`"error"` / `"warn"` / `"info"` / unset) collapses to the Info default
//! (`verbose_until = None`). Produces the schema-4 shape ([`AppConfigV4`]),
//! which drops `log_level` (consumed here). `m0005` reads V4 as its source.
//! Bumps `schema_version` to 4. See RFC 0055.
//!
//! The source is read raw off disk as
//! [`super::m0003_secure_screen_mode::AppConfigV3`]. The conversion is one-way
//! (only `"debug"` is honored), so there is no value-flip window to race.
//!
//! Idempotent (the engine gates on `schema_version`).

use rustpass::{Error, LockMode};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::app_config::{
    SecureScreenMode, VERBOSE_WINDOW_SECS, default_autosync_true, is_autosync_default, is_false,
    now_unix,
};
use crate::migrations::MigrationOutcome;
use crate::migrations::m0003_secure_screen_mode::AppConfigV3;

/// Serde default for [`AppConfigV4::schema_version`] — `4`, the version this
/// migration produces.
fn default_schema_v4() -> u32 {
    4
}

/// The on-disk `app.json` shape at schema 4: the deprecated `log_level` is gone
/// (consumed by this migration), leaving only the runtime fields.
/// **Distinct from the merge-view `AppConfig`** — V4 is the *single-file*
/// shape that `m0005` reads as its source; `AppConfig` is the *merged IPC view*
/// of the post-split `PrefConfig` + `BehaviorConfig`. They happen to carry the
/// same fields, but the schema-default differs (V4 defaults to 4 via
/// [`default_schema_v4`]; `AppConfig` defaults to 1 via the shared
/// `default_schema_version`), so they stay separate types.
///
/// Does NOT carry `#[serde(deny_unknown_fields)]` — main shipped schema-4
/// `app.json` files that still carry the deprecated `secure_screen` bool and/or
/// `log_level` (cleared by main's m0004 only when the conversion fired) must
/// upgrade cleanly. Serde ignores unknown keys, so V4 reads those files without
/// error; the deprecated keys just don't reach the runtime types.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct AppConfigV4 {
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
    pub(crate) verbose_until: Option<u64>,
    #[serde(default = "default_schema_v4")]
    pub(crate) schema_version: u32,
}

/// Manual `Default` so `autosync` matches its serde default (`true`), not the
/// derived `bool` default (`false`). Lets tests spread `..Default::default()`
/// over a partial V4 literal without silently flipping autosync off.
impl Default for AppConfigV4 {
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
            verbose_until: None,
            schema_version: default_schema_v4(),
        }
    }
}

/// Carry a pinned `"debug"` level into `verbose_until` and bump `schema_version`
/// to 4. See the module docs for the mapping. m0004's target is the V4
/// single-file snapshot ([`AppConfigV4`]); `m0005` then splits V4 into the
/// composite `PrefConfig` + sealed-`BehaviorConfig` pair.
pub(crate) async fn apply(state: &AppState, version: u32) -> Result<MigrationOutcome, Error> {
    // Read the schema-3 app.json. The engine only runs us when peek saw a
    // schema < 4 (so the file exists); a full V3 parse failure (a wrong-type
    // field that peek still parses) is rare — fall back to the V3 defaults so
    // the chain heals the file to target instead of warn-looping every launch.
    let v3: AppConfigV3 = state
        .app_config
        .read_app_json_as()
        .await
        .unwrap_or_else(|e| {
            log::warn!("0004_verbose_from_debug: app.json unparseable ({e}); using V3 defaults");
            AppConfigV3::default()
        });
    // Only a previously pinned "debug" carries into verbose; error/warn/info and
    // unset all collapse to the Info default. Leave `verbose_until` untouched if
    // a partially-migrated file already set one.
    let verbose_until = if v3.verbose_until.is_some() {
        v3.verbose_until
    } else if v3.log_level.as_deref() == Some("debug") {
        Some(now_unix() + VERBOSE_WINDOW_SECS)
    } else {
        None
    };
    let v4 = AppConfigV4 {
        secure_screen_mode: v3.secure_screen_mode,
        locale: v3.locale,
        theme_mode: v3.theme_mode,
        lock_mode: v3.lock_mode,
        view_clear_secs: v3.view_clear_secs,
        clipboard_clear_secs: v3.clipboard_clear_secs,
        autosync: v3.autosync,
        biometric_app_lock: v3.biometric_app_lock,
        verbose_until,
        schema_version: version,
    };
    state.app_config.write_app_json_raw(&v4).await?;
    Ok(MigrationOutcome::Done)
}
