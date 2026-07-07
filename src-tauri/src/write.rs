// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Secret writes & sync — the write/sync side of the store.
//!
//! Writes are **local-only** in `rustpass` (`Store::set`/`create`/`update`/
//! `delete` encrypt → write → local commit, no network). This module wraps each
//! save in the per-device autosync policy via [`Store::autosync_write`]: a
//! pull → write → push when `autosync` is on (the default), or a plain local
//! commit when it's off. The pull phase is cancellable through the global cancel
//! slot (mirroring `pull_repo`); the push is not yet cancellable.
//!
//! ## Outcome shape
//!
//! The orchestrator returns a [`WriteOutcome`]: [`WriteOutcome::Written`] on a
//! normal save, [`WriteOutcome::NeedsDivergenceResolve`] when the push was
//! rejected (a race — the remote moved during the write; the carried
//! `SyncDivergence` lets the UI show the resolve modal without a second
//! round-trip), or [`WriteOutcome::AuthenticityBlocked`] when the pre-write pull
//! was refused under Enforce signature verification. The frontend's shared
//! divergence modal routes a `NeedsDivergenceResolve` to [`resolve_sync_divergence`].
//!
//! ## Immediate-mode wipe (D3)
//!
//! `do_save`/`delete_secret` reset the auto-lock timer on every attempt, but
//! wipe the identity only on **terminal** outcomes — a `NeedsDivergenceResolve`
//! still needs the cached identity for a keep-mine resolve (`resolve_keep_mine`
//! re-encrypts local blobs), so wiping it before the user picks would force a
//! second unlock. The deferred wipe runs in [`resolve_sync_divergence`] once the
//! resolve settles.
//!
//! [`Store::autosync_write`]: rustpass::Store::autosync_write

use std::collections::HashMap;
use std::future::Future;

use rustpass::template::{self, CreatePreset};
use rustpass::{
    DivergenceChoice, Error, ErrorCode, SyncOutcome, SyncResult, WriteOutcome, WriteResult,
};
use tauri::{AppHandle, Runtime, State};

use crate::AppState;
use crate::identity::{maybe_soft_wipe, reset_lock_timer};

/// Run a local-only write under the autosync orchestrator, with the pull phase
/// cancellable via the global cancel slot (mirrors `pull_repo`). Returns the
/// orchestrator's [`WriteOutcome`] directly; the caller adds the auto-lock side
/// effects. The closure runs inside the orchestrator's `write_mu` critical
/// section and must be one of the local-only primitives (`Store::create`/
/// `update`/`delete`) — it must NOT re-acquire the Store lock.
async fn autosync_write_command<R, F, Fut>(
    state: &State<'_, AppState>,
    app: &AppHandle<R>,
    local_write: F,
) -> Result<WriteOutcome, Error>
where
    R: Runtime,
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<WriteResult, Error>> + Send + 'static,
{
    let store = state.store.clone();
    crate::git::run_cancellable(state, app.clone(), move |cancel, _tx| {
        let store = store.clone();
        async move { store.autosync_write(Some(cancel), local_write).await }
    })
    .await
}

/// Wrap a local-only save in autosync + the auto-lock side effects (reset the
/// idle timer; soft-wipe the identity under Immediate — but only on terminal
/// outcomes, per D3). The orchestrator's [`WriteOutcome`] is passed through
/// unchanged so the frontend can route `NeedsDivergenceResolve` /
/// `AuthenticityBlocked` to their modals.
async fn do_save<R, F, Fut>(
    state: &State<'_, AppState>,
    app: &AppHandle<R>,
    local_write: F,
) -> Result<WriteOutcome, Error>
where
    R: Runtime,
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<WriteResult, Error>> + Send + 'static,
{
    // Run the write first so a FAILED save still counts as a secret access: under
    // Immediate we reset the timer + wipe on the terminal paths (an errored save
    // must not leave the identity cached with no idle timer to eventually clear
    // it).
    let outcome = autosync_write_command(state, app, local_write).await;
    reset_lock_timer(state, app);
    // D3: a NeedsDivergenceResolve still needs the cached identity for a keep-mine
    // resolve, so defer the wipe to resolve_sync_divergence. Every other outcome
    // (Written / AuthenticityBlocked / Err) is terminal — wipe now.
    if !matches!(&outcome, Ok(WriteOutcome::NeedsDivergenceResolve(_))) {
        maybe_soft_wipe(state, app).await;
    }
    outcome
}

/// List the built-in secret-creation presets (Website login, PIN code) — the
/// "create from a few options" set the wizard offers.
#[tauri::command]
pub(crate) async fn list_create_presets() -> Vec<CreatePreset> {
    template::builtin_presets().to_vec()
}

/// Look up the `.pass-template` that would apply to `name`, if any. Used by the
/// wizard to hint that a template will shape the new secret.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn lookup_template(
    state: State<'_, AppState>,
    name: String,
) -> Result<Option<String>, Error> {
    state.store.lookup_template(&name).await
}

/// Preview what [`rustpass::Store::create`] would store for `name` + `content`:
/// the rendered template body, or `None` when no template applies. Writes
/// nothing.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn preview_create(
    state: State<'_, AppState>,
    name: String,
    content: String,
) -> Result<Option<String>, Error> {
    state.store.preview_create(&name, content.as_bytes()).await
}

/// Create a secret at an explicit path from its raw content (first line is the
/// password). A matching `.pass-template` is applied automatically.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn create_secret(
    state: State<'_, AppState>,
    app: AppHandle,
    name: String,
    content: String,
) -> Result<WriteOutcome, Error> {
    let store = state.store.clone();
    let body = content.into_bytes();
    do_save(&state, &app, move || {
        let store = store.clone();
        async move { store.create(&name, &body).await }
    })
    .await
}

/// Create a secret from one of the built-in presets, generating it at the
/// preset's fixed path from a few field values (Website → `websites/…`,
/// PIN → `pin/…`).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn create_from_preset_secret(
    state: State<'_, AppState>,
    app: AppHandle,
    preset_id: String,
    fields: HashMap<String, String>,
) -> Result<WriteOutcome, Error> {
    let preset = template::find_preset(&preset_id).ok_or_else(|| {
        Error::new(
            ErrorCode::InvalidEntryName,
            format!("unknown create preset: {preset_id:?}"),
        )
    })?;
    // Tauri hands us HashMap<String, String>; the template helpers key off the
    // preset's `&'static str` field keys, so rebuild as HashMap<&str, String>.
    let fields_ref: HashMap<&str, String> = fields
        .iter()
        .map(|(k, v)| (k.as_str(), v.clone()))
        .collect();
    let name = template::preset_name(preset, &fields_ref)?;
    let body = template::preset_body(preset, &fields_ref)?;
    let store = state.store.clone();
    do_save(&state, &app, move || {
        let store = store.clone();
        async move { store.create(&name, &body).await }
    })
    .await
}

/// Delete a secret at an explicit path. The entry is removed and the removal is
/// committed locally, then published by the autosync orchestrator (pull →
/// delete → push when autosync is on; local-only when off). Returns the
/// [`WriteOutcome`] — usually `Written`, or `NeedsDivergenceResolve` when the
/// delete's push lost a race (the frontend routes that to the shared modal).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn delete_secret(
    state: State<'_, AppState>,
    app: AppHandle,
    name: String,
) -> Result<WriteOutcome, Error> {
    let store = state.store.clone();
    let outcome = autosync_write_command(&state, &app, move || {
        let store = store.clone();
        async move { store.delete(&name).await }
    })
    .await;
    // Reset the auto-lock timer on the user's activity whether or not the delete
    // succeeded (mirrors the save path). Delete carries no plaintext and doesn't
    // cache the identity, so no maybe_soft_wipe coupling here — a keep-mine
    // resolve after a delete-triggered divergence re-auths via runWithAuth.
    reset_lock_timer(&state, &app);
    outcome
}

/// Edit a secret at an explicit path from its raw content (first line is the
/// password). The existing entry is overwritten in place — no `.pass-template`
/// is re-applied (templates shape new secrets, not mutations). If the entry
/// doesn't exist, [`ErrorCode::EntryNotFound`] is returned (edit can't create a
/// stray entry).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn edit_secret(
    state: State<'_, AppState>,
    app: AppHandle,
    name: String,
    content: String,
) -> Result<WriteOutcome, Error> {
    let store = state.store.clone();
    let body = content.into_bytes();
    do_save(&state, &app, move || {
        let store = store.clone();
        async move { store.update(&name, &body).await }
    })
    .await
}

/// Pull latest changes from the remote. Returns a `SyncOutcome`: a normal
/// fast-forward, or `Diverged` when local/remote have diverged (the frontend
/// shows a resolution modal). Emits `"git-progress"` events and is cancellable
/// via `cancel_git` while the fetch runs.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn pull_repo(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<SyncOutcome, Error> {
    let store = state.store.clone();
    crate::git::run_cancellable(&state, app, move |cancel, tx| async move {
        store.sync_with(Some(cancel), Some(tx)).await
    })
    .await
}

/// Manual sync (pull → push) — the Sync button. Reconciles both directions in
/// one cancellable, progress-reporting op: surfaces `SyncOutcome::Diverged`
/// (from a pull-side divergence, or a push-rejection race) for the resolve
/// modal, or an Enforce block; otherwise the push publishes any local commits.
/// A missing `origin` is a no-op at both phases (local-only store).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn sync_repo(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<SyncOutcome, Error> {
    let store = state.store.clone();
    crate::git::run_cancellable(&state, app, move |cancel, tx| async move {
        store.sync_repo(Some(cancel), Some(tx)).await
    })
    .await
}

/// Push the current branch to `origin`. Used by the create flow's deferred first
/// push — called after `create_store` + `complete_setup` so the remote only
/// receives the store once its identity is durable. A missing `origin` is a
/// no-op (local-only store), mirroring `pull_repo`.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn push_repo(state: State<'_, AppState>) -> Result<(), Error> {
    state.store.push().await
}

/// Resolve a pull/sync/save divergence by applying the user's `choice` against
/// the reviewed remote tip (`expected_remote_oid`). "Cancel" is client-side —
/// the frontend just doesn't call this. Returns the post-resolve result so the
/// badge can refresh. Also performs the auto-lock side effects (this is the
/// terminal step for a deferred save-divergence, so the Immediate wipe the save
/// path skipped runs here).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn resolve_sync_divergence(
    state: State<'_, AppState>,
    app: AppHandle,
    expected_remote_oid: String,
    choice: DivergenceChoice,
) -> Result<SyncResult, Error> {
    let result = state
        .store
        .resolve_sync_divergence(&expected_remote_oid, choice)
        .await;
    reset_lock_timer(&state, &app);
    // D3: terminal step for a deferred save-divergence — do the wipe the save
    // path skipped (no-op under Idle/Never; under Immediate it clears the
    // identity kept alive across the modal for keep-mine).
    maybe_soft_wipe(&state, &app).await;
    result
}

/// Abandon a save-triggered divergence without resolving — the user dismissed
/// the resolve modal (cancel / back). Performs the Immediate-mode wipe the save
/// path deferred ([`do_save`] skips [`maybe_soft_wipe`] on a
/// [`WriteOutcome::NeedsDivergenceResolve`] so a keep-mine resolve can reuse the
/// cached identity without a second unlock): with the resolve abandoned, nothing
/// needs the identity anymore, so clear it now rather than leaving it cached
/// until the next op or an app lock. No-op under `Idle`/`Never`. A
/// sync-triggered divergence never deferred a wipe, so its cancel path does not
/// call this.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn discard_divergence(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), Error> {
    maybe_soft_wipe(&state, &app).await;
    Ok(())
}
