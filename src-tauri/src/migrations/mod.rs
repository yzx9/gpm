// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Data-driven registry of one-shot `app.json` schema migrations.
//!
//! One engine + one ordered registry + one self-contained file per migration.
//! Adding a future migration = a new `m{MMMM}_{slug}.rs` file + one row in
//! [`MIGRATIONS`] + one arm in [`apply_migration`].
//!
//! `run_app_migrations` runs as the FIRST step of `init_state` and `app_unlock`
//! (before `refresh_security_cache` / `try_identity_auto_unlock`), so the first
//! unlock sees migrated values, not the defaults. Each step is gated on the
//! on-disk [`crate::app_config::AppConfig::schema_version`], so a partial run
//! interrupted by app-lock (sealed read fails `SEAL_KEY_UNAVAILABLE`) resumes
//! where it left off on the next call.
//!
//! Migrations are idempotent (gated on `schema_version`) and safe to call on
//! every startup and `app_unlock`.

// TODO: v1.0.0 — remove this module (the registry, every `m{MMMM}_*` migration,
// `LegacyRepoConfig`, and the `schema_version` gate on `AppConfig`) once all
// users have migrated. Mirrors `run_seal_migrate_once`'s removal TODO in
// `applock.rs`. (Plain `//`, not a doc comment — free-floating reminder.)

use rustpass::Error;

use crate::AppState;

pub(crate) mod m0002_config_scope_split;
pub(crate) mod m0003_secure_screen_mode;

/// Outcome of a single migration step.
///
/// `Pending` means the step is blocked on app-lock — the sealed `repo.json`
/// read fails `SEAL_KEY_UNAVAILABLE` until biometric injects the master key.
/// The engine stops the chain and the next `app_unlock` retries from the top.
pub(crate) enum MigrationOutcome {
    Done,
    Pending,
}

/// Ordered `(target_version, display_name)` pairs. The engine runs each whose
/// target exceeds the on-disk `schema_version`, in order. The last entry's
/// version is the schema target ([`APP_CONFIG_SCHEMA_VERSION`]).
const MIGRATIONS: &[(u32, &str)] = &[
    (2, "0002_config_scope_split"),
    (3, "0003_secure_screen_mode"),
];

/// The `app.json` schema version once every registered migration has run.
/// Derived from [`MIGRATIONS`] so it never drifts from the last migration's
/// target.
pub(crate) const APP_CONFIG_SCHEMA_VERSION: u32 =
    MIGRATIONS.last().expect("MIGRATIONS non-empty").0;

/// Run every pending migration in order. See the module docs for the app-lock
/// resume semantics.
pub(crate) async fn run_app_migrations(state: &AppState) {
    for &(version, name) in MIGRATIONS {
        if state.app_config.get().schema_version >= version {
            continue; // already migrated past this step (resume / idempotent)
        }
        match apply_migration(state, version).await {
            Ok(MigrationOutcome::Done) => {
                debug_assert_eq!(state.app_config.get().schema_version, version);
            }
            Ok(MigrationOutcome::Pending) => return, // app-lock; next unlock retries
            Err(e) => {
                log::warn!("{name} migration failed: {e}");
                return; // leave schema below target so the next run retries
            }
        }
    }
}

/// Dispatch one migration by its target schema version.
async fn apply_migration(state: &AppState, version: u32) -> Result<MigrationOutcome, Error> {
    match version {
        2 => m0002_config_scope_split::apply(state, version).await,
        3 => m0003_secure_screen_mode::apply(state, version).await,
        _ => unreachable!("no migration registered for schema version {version}"),
    }
}
