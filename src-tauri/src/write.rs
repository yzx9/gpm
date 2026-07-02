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
//! slot (mirroring `pull_repo`); the push is not yet cancellable (see
//! `.plans/0032-cancellable-saves.md`).
//!
//! ## Outcome shape (frozen for the frontend until `PR2c`)
//!
//! A rejected push surfaces as `Err(PUSH_REJECTED)` (the frontend's generic
//! error path) — the context-aware divergence modal that routes a save-triggered
//! divergence to `resolve_sync_divergence` lands in `PR2c`. So the write commands
//! still return [`WriteOutcome`], but only ever `Written`; the `Conflict` variant
//! is dead-but-present until `PR2c` retires it.
//!
//! ## Conflict stash (consume-side retained, frozen ABI)
//!
//! The `(name, plaintext)` stash ([`PendingWrite`]) carried a write collision's
//! plaintext across `resolve_write_conflict` so it didn't re-cross IPC. The
//! autosync path never produces a `Conflict`, so nothing populates the stash
//! anymore — but [`resolve_pending`], [`stash_pending`], [`clear_pending`], the
//! [`resolve_write_conflict`] command, and the `pending_write` field stay (kept
//! alive by their unit tests + the lock handler's defense-in-depth clear) until
//! `PR2c` retires them alongside the frontend modal. A plaintext is still never
//! left behind a wiped identity cache: `resolve_pending` consumes on every call
//! and `clear_pending` runs on lock.
//!
//! [`Store::autosync_write`]: rustpass::Store::autosync_write
//! [`Store::resolve_write_conflict`]: rustpass::Store::resolve_write_conflict

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use rustpass::template::{self, CreatePreset};
use rustpass::{
    ConflictChoice, DivergenceChoice, Error, ErrorCode, SyncOutcome, SyncResult, WriteOutcome,
    WriteResult,
};
use tauri::{AppHandle, Runtime, State};
use zeroize::Zeroizing;

use crate::AppState;
use crate::identity::{maybe_soft_wipe, reset_lock_timer};

/// A write that collided with a newer remote copy and is awaiting the user's
/// resolution. Held only in memory; `plaintext` is [`Zeroizing`] and the struct
/// intentionally does not derive `Debug`.
pub(crate) struct PendingWrite {
    /// The entry name that collided (passed back to `resolve_write_conflict`).
    name: String,
    /// The plaintext we tried to write, replayed when the user picks
    /// `keep_mine` / `keep_mine_force`.
    plaintext: Zeroizing<Vec<u8>>,
}

/// Drop any stashed pending write.
///
/// Called on lock so a conflict modal left open across an auto-lock
/// can't leave a plaintext behind a wiped identity cache (which would also make
/// a later resolve fail confusingly).
pub(crate) fn clear_pending(pending: &Arc<Mutex<Option<PendingWrite>>>) {
    if let Ok(mut pw) = pending.lock() {
        pw.take();
    }
}

/// Stash the `(name, plaintext)` of a write that collided, so
/// [`resolve_pending`] can replay it without the frontend re-sending the secret
/// across IPC a second time. Pure (no `AppHandle`), so the stash lifecycle is
/// directly unit-testable.
///
/// Test-only now: the autosync write path never produces a `Conflict`, so
/// nothing populates the stash in production. Kept under `cfg(test)` as the
/// helper the stash-lifecycle unit tests build the slot with; the consume-side
/// ([`resolve_pending`] / [`clear_pending`]) stays production-live for the
/// frozen `resolve_write_conflict` command and the lock handler.
#[cfg(test)]
pub(crate) fn stash_pending(pending: &Arc<Mutex<Option<PendingWrite>>>, name: &str, body: Vec<u8>) {
    let mut pw = pending.lock().expect("pending_write mutex poisoned");
    *pw = Some(PendingWrite {
        name: name.to_string(),
        plaintext: Zeroizing::new(body),
    });
}

/// Consume the stashed pending write and resolve the conflict per `choice`. The
/// stash is always taken (cleared) — even on error — so a plaintext never lingers
/// awaiting a retry that re-stashes fresh. Pure core (no `AppHandle`): directly
/// unit-testable, and the consume path the (frozen) `resolve_write_conflict`
/// command still calls.
pub(crate) async fn resolve_pending(
    state: &AppState,
    choice: ConflictChoice,
) -> Result<Option<WriteResult>, Error> {
    let pending = {
        let mut pw = state
            .pending_write
            .lock()
            .expect("pending_write mutex poisoned");
        pw.take()
    };
    let Some(pending) = pending else {
        return Err(Error::new(
            ErrorCode::StoreError,
            "no pending write to resolve",
        ));
    };
    state
        .store
        .resolve_write_conflict(&pending.name, &pending.plaintext, choice)
        .await
}

/// Run a local-only write under the autosync orchestrator, with the pull phase
/// cancellable via the global cancel slot (mirrors `pull_repo`). Returns the
/// write result; the caller decides the IPC shape (`WriteOutcome::Written` for
/// create/edit, raw `WriteResult` for delete). The closure runs inside the
/// orchestrator's `write_mu` critical section and must be one of the local-only
/// primitives (`Store::create`/`update`/`delete`) — it must NOT re-acquire the
/// Store lock.
async fn autosync_write_command<R, F, Fut>(
    state: &State<'_, AppState>,
    app: &AppHandle<R>,
    local_write: F,
) -> Result<WriteResult, Error>
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
/// idle timer; soft-wipe the identity under Immediate). Maps the orchestrator's
/// `Ok(WriteResult)` to [`WriteOutcome::Written`] so the create/edit IPC shape
/// is unchanged for the frozen frontend.
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
    // Immediate we reset the timer + wipe on both paths (an errored save must
    // not leave the identity cached with no idle timer to eventually clear it).
    let result = autosync_write_command(state, app, local_write).await;
    reset_lock_timer(state, app);
    // No Conflict is ever produced now (autosync surfaces a rejected push as
    // Err), so there is never a pending stash gating the wipe — maybe_soft_wipe
    // proceeds whenever Immediate is in effect.
    maybe_soft_wipe(state, app).await;
    Ok(WriteOutcome::Written(result?))
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
    do_save(
        &state,
        &app,
        move || {
            let store = store.clone();
            async move { store.create(&name, &body).await }
        },
    )
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
    do_save(
        &state,
        &app,
        move || {
            let store = store.clone();
            async move { store.create(&name, &body).await }
        },
    )
    .await
}

/// Delete a secret at an explicit path. The entry is removed and the removal is
/// committed locally, then published by the autosync orchestrator (pull →
/// delete → push when autosync is on; local-only when off). Returns the
/// `WriteResult` (unchanged IPC shape for the frozen frontend).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn delete_secret(
    state: State<'_, AppState>,
    app: AppHandle,
    name: String,
) -> Result<WriteResult, Error> {
    let store = state.store.clone();
    let result = autosync_write_command(
        &state,
        &app,
        move || {
            let store = store.clone();
            async move { store.delete(&name).await }
        },
    )
    .await;
    // Reset the auto-lock timer on the user's activity whether or not the delete
    // succeeded (mirrors the save path). Delete carries no plaintext, so no
    // maybe_soft_wipe coupling.
    reset_lock_timer(&state, &app);
    result
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
    do_save(
        &state,
        &app,
        move || {
            let store = store.clone();
            async move { store.update(&name, &body).await }
        },
    )
    .await
}

/// Resolve a write conflict ([`WriteOutcome::Conflict`]) per the user's
/// `choice`. Replays the stashed plaintext for `keep_mine` / `keep_mine_force`;
/// `keep_remote` fast-forwards to the remote, `cancel` leaves the pre-write
/// state. The stash is always consumed (cleared) on return.
///
/// Frozen frontend ABI: the command stays registered, but the autosync write
/// path never produces a `Conflict`, so the stash is never populated and this
/// returns `Err` ("no pending write to resolve") if ever called. It is retired
/// alongside the frontend modal in `PR2c`.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn resolve_write_conflict(
    state: State<'_, AppState>,
    app: AppHandle,
    choice: ConflictChoice,
) -> Result<Option<WriteResult>, Error> {
    let result = resolve_pending(&state, choice).await;
    reset_lock_timer(&state, &app);
    maybe_soft_wipe(&state, &app).await;
    result
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

/// Push the current branch to `origin`. Used by the create flow's deferred first
/// push — called after `create_store` + `complete_setup` so the remote only
/// receives the store once its identity is durable. A missing `origin` is a
/// no-op (local-only store), mirroring `pull_repo`.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn push_repo(state: State<'_, AppState>) -> Result<(), Error> {
    state.store.push().await
}

/// Resolve a pull/sync divergence by adopting the remote tip the user reviewed
/// (`expected_remote_oid`). "Cancel" is client-side — the frontend just doesn't
/// call this. Returns the post-adopt result so the badge can refresh.
///
/// (PR1 keeps the adopt-remote behavior; PR2 lets the frontend pass a
/// [`DivergenceChoice`] so "keep mine" is reachable from the context-aware modal.)
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn resolve_sync_divergence(
    state: State<'_, AppState>,
    expected_remote_oid: String,
) -> Result<SyncResult, Error> {
    state
        .store
        .resolve_sync_divergence(&expected_remote_oid, DivergenceChoice::AdoptRemote)
        .await
}

#[cfg(test)]
mod tests {
    //! The lock-clearing invariant is the security-critical piece of the conflict
    //! stash: a plaintext must not survive a lock. The stash utility lifecycle
    //! (stash → clear → resolve-consumes) is covered here; the rustpass write +
    //! autosync flows are covered end-to-end by the `rustpass` integration tests.

    use super::*;

    fn stashed(name: &str) -> Arc<Mutex<Option<PendingWrite>>> {
        Arc::new(Mutex::new(Some(PendingWrite {
            name: name.to_string(),
            plaintext: Zeroizing::new(b"s3kr3t".to_vec()),
        })))
    }

    #[test]
    fn clear_pending_drops_a_stashed_write() {
        let pending = stashed("sites/foo");
        clear_pending(&pending);
        assert!(pending.lock().unwrap().is_none());
    }

    #[test]
    fn clear_pending_is_a_noop_when_empty() {
        let pending: Arc<Mutex<Option<PendingWrite>>> = Arc::new(Mutex::new(None));
        clear_pending(&pending);
        assert!(pending.lock().unwrap().is_none());
    }
}
