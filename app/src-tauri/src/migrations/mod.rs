// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Data-driven registry of one-shot `app.json`/`pref.json` schema migrations.
//!
//! One engine + one ordered registry + one self-contained file per migration.
//! Adding a future migration = a new `m{MMMM}_{slug}.rs` file + one row in
//! [`MIGRATIONS`] + one arm in [`apply_migration`].
//!
//! `run_app_migrations` runs as the FIRST step of `init_state` and `app_unlock`
//! (before `refresh_security_cache` / `try_identity_auto_unlock`), so the first
//! unlock sees migrated values, not the defaults. Each step is gated on the
//! on-disk `schema_version` (peeked raw — see
//! [`crate::app_config::AppConfigStore`]), NOT the in-memory cache: migrations
//! write raw (no per-step cache swap), so the cache is intentionally stale
//! mid-chain and reloaded once at the end. A partial run interrupted by app-lock
//! (sealed read fails `SEAL_KEY_UNAVAILABLE`) resumes where it left off on the
//! next call.
//!
//! Migrations are idempotent (gated on `schema_version`) and safe to call on
//! every startup and `app_unlock`.

use rustpass::{Error, ErrorCode};

use crate::AppState;
use crate::app_config::PeekOutcome;

pub(crate) mod m0002_config_scope_split;
pub(crate) mod m0003_secure_screen_mode;
pub(crate) mod m0004_verbose_from_debug;
pub(crate) mod m0005_split_app_json;
pub(crate) mod m0006_gate_idle_timeout;
pub(crate) mod m0007_vault_key;
pub(crate) mod m0008_collapse_pref_into_sealed;
pub(crate) mod m0009_multi_repo_register;
pub(crate) mod m0010_multi_repo_relocate;

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
    (4, "0004_verbose_from_debug"),
    (5, "0005_split_app_json"),
    (6, "0006_gate_idle_timeout"),
    (7, "0007_vault_key"),
    (8, "0008_collapse_pref_into_sealed"),
    (9, "0009_multi_repo_register"),
    (10, "0010_multi_repo_relocate"),
];

/// The `app.json`/`pref.json` schema version once every registered migration has
/// run. Derived from [`MIGRATIONS`] so it never drifts from the last migration's
/// target.
pub(crate) const APP_CONFIG_SCHEMA_VERSION: u32 =
    MIGRATIONS.last().expect("MIGRATIONS non-empty").0;

/// Run every pending migration in order. See the module docs for the app-lock
/// resume semantics.
pub(crate) async fn run_app_migrations(state: &AppState) {
    // Gate off a raw on-disk peek (3-state PeekOutcome), NOT the in-memory cache:
    // migrations write raw (no per-step cache swap), so the cache is stale
    // mid-chain. R074 moved `schema_version` into the sealed merged `app.json`,
    // so the peek unseals to read it (the auth-free key is loaded at `.setup()`,
    // so a present file always unseals — no "deferred" state).
    let mut current = match state.app_config.peek_schema_version().await {
        PeekOutcome::Version(v) => v,
        // Fresh install / post-reset (no config file) ⇒ skip the whole chain.
        PeekOutcome::Absent => APP_CONFIG_SCHEMA_VERSION,
        // `app.json` present but unreadable (real tamper / lost key). Halt + log
        // so the user routes to re-setup — NEVER silently treat as Absent, which
        // would wipe their prefs by skipping to defaults.
        PeekOutcome::Corrupt => {
            log::warn!(
                "app-config: sealed app.json present but unreadable (tamper/lost key); \
                 halting migrations — re-setup may be required"
            );
            return;
        }
    };
    let mut ran = false;
    for &(version, name) in MIGRATIONS {
        if current >= version {
            continue; // already migrated past this step (resume / idempotent)
        }
        match apply_migration(state, version).await {
            Ok(MigrationOutcome::Done) => {
                // The migration persisted its target version to disk.
                current = version;
                ran = true;
                debug_assert_eq!(
                    state.app_config.peek_schema_version().await,
                    PeekOutcome::Version(version)
                );
            }
            Ok(MigrationOutcome::Pending) => return, // app-lock; next unlock retries
            Err(e) => {
                log::warn!("{name} migration failed: {e}");
                return; // leave schema below target so the next run retries
            }
        }
    }
    // Re-read the migrated files into the cache so the post-migration runtime
    // reads (`effective_log_filter`, the facade autosync seed, etc.) see the
    // migrated values. No reload on Pending/Err: the next run re-peeks disk
    // and resumes. ADDITIVE — does NOT replace the standalone
    // `reload_behavior` in `init_state`/`app_unlock` + the
    // `seed_registry_facades` autosync seed, which cover the no-migration
    // cold-start path (where `ran=false`).
    if ran && let Err(e) = state.app_config.reload().await {
        log::warn!("app-config: post-migration cache reload failed: {e}");
    }
}

/// Dispatch one migration by its target schema version.
async fn apply_migration(state: &AppState, version: u32) -> Result<MigrationOutcome, Error> {
    match version {
        2 => m0002_config_scope_split::apply(state, version).await,
        3 => m0003_secure_screen_mode::apply(state, version).await,
        4 => m0004_verbose_from_debug::apply(state, version).await,
        5 => m0005_split_app_json::apply(state, version).await,
        6 => m0006_gate_idle_timeout::apply(state, version).await,
        7 => m0007_vault_key::apply(state, version).await,
        8 => m0008_collapse_pref_into_sealed::apply(state, version).await,
        9 => m0009_multi_repo_register::apply(state, version).await,
        10 => m0010_multi_repo_relocate::apply(state, version).await,
        // Unreachable in practice — `version` comes from iterating `MIGRATIONS`,
        // whose every entry has a match arm above. Return an `Err` (not a panic)
        // so a future mismatch (a registry row without a dispatch arm) surfaces
        // as a logged + retryable migration failure rather than crashing startup
        // in release builds.
        _ => Err(Error::new(
            ErrorCode::ConfigError,
            format!("no migration registered for schema version {version}"),
        )),
    }
}
