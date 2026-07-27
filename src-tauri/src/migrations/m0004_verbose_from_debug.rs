// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Migration `0004_verbose_from_debug`.
//!
//! Carries a previously pinned `"debug"` log level into the new verbose flag:
//! `log_level == "debug"` ⇒ `verbose_until = now + VERBOSE_WINDOW_SECS` (so the
//! upgrade is non-surprising — the user keeps Debug, then it expires under the
//! same time-box as any other verbose session); every other `log_level`
//! (`"error"` / `"warn"` / `"info"` / unset) collapses to the Info default
//! (`verbose_until = None`). Bumps `schema_version` to 4. See RFC 0055.
//!
//! Reads the deprecated `log_level` field directly off the cached `AppConfig`
//! — it is kept as a deprecated serde field through this transition (removed at
//! v1.0.0), the same shape `m0003` uses for the `secure_screen` bool. No
//! `raw_at_load` snapshot is needed: the conversion is one-way (only `"debug"`
//! is honored), so there is no value-flip window to race.
//!
//! Idempotent (the engine gates on `schema_version`).

use rustpass::Error;

use crate::AppState;
use crate::app_config::{VERBOSE_WINDOW_SECS, now_unix};
use crate::migrations::MigrationOutcome;

/// Carry a pinned `"debug"` level into `verbose_until` and bump `schema_version`
/// to 4. See the module docs for the mapping and the deprecated-field rationale.
pub(crate) async fn apply(state: &AppState, version: u32) -> Result<MigrationOutcome, Error> {
    let mut cfg = state.app_config.get();
    // Only a previously pinned "debug" carries into verbose; error/warn/info and
    // unset all collapse to the Info default. Leave `verbose_until` untouched if
    // a partially-migrated file already set one.
    if cfg.verbose_until.is_none() && cfg.log_level.as_deref() == Some("debug") {
        cfg.verbose_until = Some(now_unix() + VERBOSE_WINDOW_SECS);
    }
    // The deprecated field has now been considered (carried or collapsed), so
    // drop it — a leftover `"debug"` would silently resurrect Debug logging on a
    // downgrade to a pre-verbose build (schema already 4 ⇒ m0004 won't re-run).
    // Unlike m0003's `secure_screen` bool, this is an `Option`, so clearing it
    // omits it from app.json via `skip_serializing_if`.
    cfg.log_level = None;
    cfg.schema_version = version;
    // Propagate a save failure so the engine retries — never mark Done without
    // persisting (a crash here would otherwise leave "debug" un-carried).
    state.app_config.save(&cfg).await?;
    Ok(MigrationOutcome::Done)
}
