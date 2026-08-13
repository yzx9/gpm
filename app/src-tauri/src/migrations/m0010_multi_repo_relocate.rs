// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Migration `0010_multi_repo_relocate` (R080).
//!
//! The **relocate half** of multi-repository: move the single registered
//! repository's files from the device config root (`config_dir/`) into its
//! per-repo subdirectory `config_dir/repositories/<id>/`, and rewrite
//! `RepoConfig.local_path` to the clone's new location. After this the registry
//! facade (built at `config_dir/repositories/<id>/` by `init_state`) diverges
//! from the device facade (`config_dir`, which owns only `app.json`) — the
//! structural split C2a's active-facade routing was preparing for.
//!
//! m0009 (register) must have run first: it assigned the id this migration
//! reads back from `app.json` (the id is **never regenerated** here). A fresh
//! install (empty `repositories`) has nothing to relocate — bump the schema and
//! return; a repo added later by setup is relocated by `register_first_repo` at
//! adoption time.
//!
//! Crash-atomicity: the physical work (file/dir renames + the `local_path`
//! rewrite) is done by [`crate::relocate_repo_into_subdir`], which is idempotent
//! (each file moves only while still at the config root; a prior partial run's
//! moves are skipped). The schema bump to 10 is the commit point — a crash
//! before it leaves schema < 10, so the engine re-runs m0010 on the next launch,
//! the idempotent relocate completes the remaining moves, and the schema lands.
//! No half-repo, no duplication, no data loss.
//!
//! Like m0008/m0009, only the auth-free master key is touched (the `repo.json`
//! read/write is master-sealed; the key is loaded at `.setup()` per R074), so
//! this runs under App Lock too. Idempotent (gated on `schema_version`) and safe
//! to call on every startup and `app_unlock`.

use rustpass::Error;

use crate::AppState;
use crate::app_config::AppConfig;
use crate::migrations::MigrationOutcome;
use crate::registry::RepoId;

/// Relocate the single registered repository (if any) into
/// `config_dir/repositories/<id>/`, then bump `schema_version` to 10. See the
/// module docs.
///
/// Outcomes:
/// - empty `repositories` (fresh install) ⇒ no-op + schema bump.
/// - a registered repo ⇒ relocate it (idempotent), bump schema, save.
/// - unseal/move/save failure ⇒ `Err` (engine retries; persistent = re-setup).
pub(crate) async fn apply(state: &AppState, version: u32) -> Result<MigrationOutcome, Error> {
    let bytes = state.store.load_app_config().await?;
    let mut cfg: AppConfig = serde_json::from_slice(&bytes)?;

    // No registered repo (fresh install / never-completed setup) ⇒ nothing to
    // relocate. Still bump the schema so this step doesn't re-run every launch;
    // a repo added later is relocated by `register_first_repo` at adoption.
    let Some(id_str) = cfg.repositories.first().cloned() else {
        cfg.schema_version = version;
        let json = serde_json::to_string_pretty(&cfg)?;
        state.store.save_app_config(json.as_bytes()).await?;
        return Ok(MigrationOutcome::Done);
    };

    // Relocate the repo's files into `repositories/<id>/` (idempotent +
    // crash-safe; reads the id m0009 minted — never regenerates). The device
    // store (master key in hand) drives the `repo.json` read/write + derives the
    // paths from its own config root.
    let id = RepoId::from(id_str);
    crate::relocate_repo_into_subdir(&state.store, &id).await?;

    // Commit point: bump the schema. A crash before this leaves schema < 10 ⇒
    // the engine re-runs m0010 ⇒ the idempotent relocate completes ⇒ schema lands.
    cfg.schema_version = version;
    let json = serde_json::to_string_pretty(&cfg)?;
    state.store.save_app_config(json.as_bytes()).await?;
    Ok(MigrationOutcome::Done)
}
