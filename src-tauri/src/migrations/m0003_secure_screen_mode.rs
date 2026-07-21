// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Migration `0003_secure_screen_mode`.
//!
//! Converts the deprecated boolean `secure_screen` toggle into the three-state
//! `secure_screen_mode`: `false → Off`, `true`/missing → `None` (the `Sensitive`
//! default, so a default user's on-disk shape stays byte-identical to a fresh
//! install). Bumps `schema_version` to 3.
//!
//! The old bool is read directly off the cached `AppConfig` — it is kept as a
//! deprecated serde field through this transition (removed at v1.0.0), so the
//! round-trip save in `0002_config_scope_split` does not purge it before this
//! step runs. No `raw_at_load` snapshot is needed: that approach had a
//! crash/save-failure window that could silently flip `Off → Sensitive`.
//!
//! Idempotent (the engine gates on `schema_version`).

use rustpass::Error;

use crate::AppState;
use crate::app_config::SecureScreenMode;
use crate::migrations::MigrationOutcome;

/// Convert the deprecated `secure_screen` bool into `secure_screen_mode` and
/// bump `schema_version` to 3. See the module docs for the mapping and the
/// rationale for reading the bool directly instead of via a load snapshot.
pub(crate) async fn apply(state: &AppState, version: u32) -> Result<MigrationOutcome, Error> {
    let mut cfg = state.app_config.get();
    // Only set the mode when it is not already pinned (a partially-migrated
    // file re-running this step keeps any explicit value). false → Off;
    // true/missing → None — None is the Sensitive default, so a default user's
    // app.json stays byte-identical.
    if cfg.secure_screen_mode.is_none() {
        cfg.secure_screen_mode = (!cfg.secure_screen).then_some(SecureScreenMode::Off);
    }
    cfg.schema_version = version;
    // Propagate a save failure so the engine retries — never mark Done without
    // persisting (a crash here would otherwise leave the bool unconverted).
    state.app_config.save(&cfg).await?;
    Ok(MigrationOutcome::Done)
}
