// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Migration `0009_multi_repo_register` (R080).
//!
//! The **register half** of multi-repository: adopt the existing single
//! repository (if any) into the new `AppConfig` registry — assign it a stable,
//! opaque `RepoId` and persist `repositories` + `last_active` into the sealed
//! merged `app.json`. **No files move** and **no facade is re-rooted**: the
//! repository stays at `config_dir/` exactly where it is today. The physical
//! relocation into `config_dir/repositories/<id>/` is a later migration; until
//! then the registry's facade is the same `config_dir`-rooted store.
//!
//! Why split register from relocate: the registry must hold a real id BEFORE any
//! command can resolve `state.registry.facade(repoId)`, so the operation-surface
//! threading can land while facades are still `config_dir`-rooted (no big-bang
//! relocate + re-thread at once). Each step stays green.
//!
//! Algorithm:
//!
//! ```text
//! unseal app.json@8 ─► AppConfig ─► repositories empty? ─no─► (already registered) ─► bump schema, Done
//!                                        │yes
//!                                        ├─ repo.json valid at config_dir? ─yes─► assign id,
//!                                        │                                        repositories=[id], last_active=id
//!                                        │
//!                                        └─ no repo (fresh / never-completed setup) ─► registry stays empty
//!                                     bump schema_version to 9 ─► save sealed merged app.json ─► Done
//! ```
//!
//! - A valid existing repository (`Store::config()` unseals `repo.json` ⇒ `Ok`)
//!   gets a freshly generated id. `repo.json` is master-sealed (auth-free key,
//!   loaded at `.setup()` per R074/D), so this reads it even under App Lock.
//! - A fresh / never-set-up install (`Store::config()` ⇒ `NO_REPO`) leaves the
//!   registry empty; first-run setup registers the repo directly (no migration
//!   needed).
//! - A corrupt `repo.json` (any error other than `NO_REPO`) ⇒ `Err`: the engine
//!   halts and retries on the next run rather than silently dropping the user's
//!   repo from the registry.
//! - The sealed merged `app.json` write (temp + rename, atomic) is the commit.
//!
//! **No app-lock defer**: like `m0008`, only the auth-free master key is touched.
//! Idempotent (gated on `schema_version`) and safe to call on every startup and
//! `app_unlock`.

use rustpass::Error;

use crate::AppState;
use crate::app_config::AppConfig;
use crate::migrations::MigrationOutcome;
use crate::registry::RepoId;

/// Assign a stable `RepoId` to the existing single repository (if present) and
/// persist `repositories` + `last_active` into the sealed merged `app.json`,
/// bumping `schema_version` to 9. See the module docs.
///
/// Outcomes:
/// - `app.json` parses as `AppConfig`@<9 with repositories already populated ⇒
///   defensive no-op re-save at schema 9 (a prior run registered but, impossibly,
///   crashed before the schema bump — the engine gates on schema, so unreachable).
/// - a valid `repo.json` at `config_dir` ⇒ register it, bump schema, save.
/// - no valid repo (fresh install) ⇒ leave the registry empty, bump schema, save.
/// - unseal/parse/save failure ⇒ `Err` (engine retries; persistent = re-setup).
pub(crate) async fn apply(state: &AppState, version: u32) -> Result<MigrationOutcome, Error> {
    // The merged `app.json` (master-sealed; the auth-free key is loaded at
    // `.setup()`). The engine only calls us when on-disk schema < 9, so this is
    // the schema-8 shape (repositories/last_active absent ⇒ defaulted empty).
    let bytes = state.store.load_app_config().await?;
    let mut cfg: AppConfig = serde_json::from_slice(&bytes)?;

    // Adopt the legacy single repository into the registry iff one exists at
    // config_dir AND the registry is not already populated. `Store::config()`
    // unseals repo.json (master-sealed): Ok ⇒ a valid repo is present (adopt it);
    // `NO_REPO` ⇒ none (fresh / never-completed setup — registry stays empty,
    // first-run setup registers later); any other error ⇒ a corrupt repo.json —
    // propagate so the engine halts + retries instead of silently dropping the
    // user's repo and bumping the schema past the recovery point.
    if cfg.repositories.is_empty() {
        match state.store.config().await {
            Ok(_) => {
                let id = RepoId::generate()?;
                let id_str = id.to_string();
                cfg.repositories = vec![id_str.clone()];
                cfg.last_active = Some(id_str);
            }
            Err(e) if e.code == "NO_REPO" => {}
            Err(e) => return Err(e),
        }
    }

    cfg.schema_version = version;
    let json = serde_json::to_string_pretty(&cfg)?;
    state.store.save_app_config(json.as_bytes()).await?;
    Ok(MigrationOutcome::Done)
}
