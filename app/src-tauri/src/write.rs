// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Secret writes & sync — the write/sync side of the store.
//!
//! Writes are **local-only** in `rustpass` (`Store::set`/`create`/`update`/
//! `delete` encrypt → write → local commit, no network). This module wraps each
//! save in the per-device autosync policy via [`Store::autosync_write`]: a
//! pull → write → push when `autosync` is on (the default), or a plain local
//! commit when it's off. Both the pull and push phases are cancellable through
//! the shared cancel slot (armed under `write_mu`); a cancelled save surfaces as
//! `WriteOutcome::Cancelled { committed }`.
//!
//! ## Outcome shape
//!
//! The orchestrator returns a [`WriteOutcome`]: [`WriteOutcome::Written`] on a
//! normal save, [`WriteOutcome::NeedsDivergenceResolve`] when the push was
//! rejected (a race — the remote moved during the write; the carried
//! `SyncDivergence` lets the UI show the resolve modal without a second
//! round-trip), [`WriteOutcome::AuthenticityBlocked`] when the pre-write pull
//! was refused under Enforce signature verification, or
//! [`WriteOutcome::EntryConflict`] / [`WriteOutcome::NoChange`] when a
//! base-version-aware edit/delete refused a stale write (R026). The frontend's
//! divergence modal routes a `NeedsDivergenceResolve` to
//! [`resolve_sync_divergence`]; an `EntryConflict` routes to
//! [`resolve_entry_conflict`].
//!
//! ## Immediate-mode wipe
//!
//! `do_save`/`delete_secret` reset the auto-lock timer on every attempt, but
//! wipe the identity only on **terminal** outcomes — a `NeedsDivergenceResolve`
//! or `EntryConflict` still needs the cached identity for a keep-mine resolve
//! (`resolve_keep_mine` re-encrypts local blobs; a keep-mine edit re-encrypts the
//! caller's body), so wiping before the user picks would force a second unlock.
//! The deferred wipe runs in [`resolve_sync_divergence`] /
//! [`resolve_entry_conflict`] once the resolve settles.
//!
//! [`Store::autosync_write`]: rustpass::Store::autosync_write

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustpass::template::{self, CreatePreset};
use rustpass::{
    DivergenceChoice, EntryConflictChoice, Error, ErrorCode, ExpectedEntry, ExpectedKind,
    SyncOutcome, SyncResult, WriteOutcome, WriteResult,
};
use tauri::{AppHandle, Emitter, Runtime, State};

use crate::AppState;
use crate::identity::{maybe_soft_wipe, reset_gate_idle_timer, reset_lock_timer};

/// Hard deadline (seconds) for a best-effort background sync. A companion task
/// flips the background sync's private cancel token at this point so a stalled or
/// malicious remote can't hold `write_mu` indefinitely and queue every user
/// save/Sync behind a headless, non-user-initiated op.
const BACKGROUND_SYNC_DEADLINE_SECS: u64 = 30;

/// Run a local-only write under the autosync orchestrator, with the pull phase
/// cancellable via the global cancel slot (mirrors `pull_repo`). Returns the
/// orchestrator's [`WriteOutcome`] directly; the caller adds the auto-lock side
/// effects. The closure runs inside the orchestrator's `write_mu` critical
/// section and must be one of the local-only primitives (`Store::create`/
/// `update`/`delete`) — it must NOT re-acquire the Store lock.
async fn autosync_write_command<R, F, Fut>(
    state: &State<'_, AppState>,
    app: &AppHandle<R>,
    expected: Option<ExpectedEntry>,
    local_write: F,
) -> Result<WriteOutcome, Error>
where
    R: Runtime,
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<WriteResult, Error>> + Send + 'static,
{
    let store = state.store.clone();
    crate::git::run_cancellable(state, app.clone(), move |cancel, _tx, slot| {
        let store = store.clone();
        async move {
            store
                .autosync_write(&slot, Some(cancel), expected, local_write)
                .await
        }
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
    expected: Option<ExpectedEntry>,
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
    let outcome = autosync_write_command(state, app, expected, local_write).await;
    reset_lock_timer(state, app);
    reset_gate_idle_timer(state, app);
    // D3: a NeedsDivergenceResolve still needs the cached identity for a keep-mine
    // resolve, so defer the wipe to resolve_sync_divergence; an EntryConflict
    // (R026) likewise keeps it cached for a keep-mine edit resolve. Every other
    // outcome (Written / AuthenticityBlocked / Err) is terminal — wipe now.
    if !matches!(
        &outcome,
        Ok(WriteOutcome::NeedsDivergenceResolve(_) | WriteOutcome::EntryConflict { .. })
    ) {
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
    log::info!("create: {name}");
    let expected = ExpectedEntry {
        name: name.clone(),
        base_oid: String::new(),
        kind: ExpectedKind::Create,
    };
    let store = state.store.clone();
    let body = content.into_bytes();
    do_save(&state, &app, Some(expected), move || {
        let store = store.clone();
        async move { store.create(&name, &body).await }
    })
    .await
    .inspect_err(|e| log::warn!("create failed: {e}"))
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
    log::info!("create: {name} (preset {preset_id})");
    let body = template::preset_body(preset, &fields_ref)?;
    let store = state.store.clone();
    // R026: preset create is NOT base-version-guarded (custom create is). The
    // keep-mine resolve would need to re-send the template-rendered body, which
    // the frontend doesn't hold (it sends fields, not the body), so a conflict
    // here can't be resolved client-side. A same-name preset collision stays a
    // documented gap (rare: two devices filling the same preset fields).
    do_save(&state, &app, None, move || {
        let store = store.clone();
        async move { store.create(&name, &body).await }
    })
    .await
    .inspect_err(|e| log::warn!("create failed: {e}"))
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
    base_oid: Option<String>,
) -> Result<WriteOutcome, Error> {
    log::info!("delete: {name}");
    let expected = base_oid.map(|base_oid| ExpectedEntry {
        name: name.clone(),
        base_oid,
        kind: ExpectedKind::Delete,
    });
    let store = state.store.clone();
    let outcome = autosync_write_command(&state, &app, expected, move || {
        let store = store.clone();
        async move { store.delete(&name).await }
    })
    .await
    .inspect_err(|e| log::warn!("delete failed: {e}"));
    // Reset the auto-lock timer on the user's activity whether or not the delete
    // succeeded (mirrors the save path). Delete carries no plaintext and doesn't
    // cache the identity, so no maybe_soft_wipe coupling here — a keep-mine
    // resolve after a delete-triggered divergence re-auths via runWithAuth.
    reset_lock_timer(&state, &app);
    reset_gate_idle_timer(&state, &app);
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
    base_oid: Option<String>,
) -> Result<WriteOutcome, Error> {
    log::info!("edit: {name}");
    let expected = base_oid.map(|base_oid| ExpectedEntry {
        name: name.clone(),
        base_oid,
        kind: ExpectedKind::Edit,
    });
    let store = state.store.clone();
    let body = content.into_bytes();
    do_save(&state, &app, expected, move || {
        let store = store.clone();
        async move { store.update(&name, &body).await }
    })
    .await
    .inspect_err(|e| log::warn!("edit failed: {e}"))
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
    log::info!("pull: start");
    let store = state.store.clone();
    crate::git::run_cancellable(&state, app, move |cancel, tx, slot| async move {
        store.sync_with(&slot, Some(cancel), Some(tx)).await
    })
    .await
    .inspect(|o| log::info!("pull: done: {o:?}"))
    .inspect_err(|e| log::warn!("pull failed: {e}"))
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
    log::info!("sync: start");
    let store = state.store.clone();
    crate::git::run_cancellable(&state, app, move |cancel, tx, slot| async move {
        store.sync_repo(&slot, Some(cancel), Some(tx)).await
    })
    .await
    .inspect(|o| log::info!("sync: done: {o:?}"))
    .inspect_err(|e| log::warn!("sync failed: {e}"))
}

/// Run a best-effort, deadline-bounded sync under a PRIVATE throwaway cancel
/// slot. Shared by the foreground cold-start/resume sync and the
/// headless background-sync JNI entry, so the deadline + private-slot logic
/// lives in one place. The private slot never touches the shared
/// `active_cancel_slot` the user's pull-to-refresh relies on. The caller passes
/// the sync op (foreground: pull+push `sync_repo`; background: pull-only
/// `sync_with`) as a closure receiving the private slot + the cancel token.
///
/// A companion task flips the token at `BACKGROUND_SYNC_DEADLINE_SECS`, capping
/// how long a stalled/malicious remote can hold `write_mu`. Uses `tokio::spawn`
/// (works on the Tauri runtime and the JNI entry's own runtime).
pub(crate) async fn run_best_effort_sync<F, Fut>(op: F) -> Result<SyncOutcome, Error>
where
    F: FnOnce(rustpass::CancelSlot, rustpass::CancelToken) -> Fut,
    Fut: Future<Output = Result<SyncOutcome, Error>>,
{
    let cancel = crate::git::fresh_cancel_token();
    let deadline = tokio::spawn({
        let cancel = Arc::clone(&cancel);
        async move {
            tokio::time::sleep(Duration::from_secs(BACKGROUND_SYNC_DEADLINE_SECS)).await;
            cancel.store(true, Ordering::SeqCst);
        }
    });
    // PRIVATE throwaway slot — never the shared `active_cancel_slot` (the user's
    // pull-to-refresh cancel). Nobody polls this slot; it just satisfies the arm.
    let private_slot: rustpass::CancelSlot = Arc::new(Mutex::new(None));
    let result = op(private_slot, cancel).await;
    deadline.abort(); // settled (ok, err, or deadline-cancelled) — stop the timer
    result
}

/// Best-effort background sync (cold-start / resume, RFC R060 Tier 1) — pull + push
/// directly, **bypassing `run_cancellable`** so it never touches the shared cancel
/// slot the user's pull-to-refresh relies on, and reporting no progress (it's a
/// headless trigger, not a user-initiated action). Returns the outcome so the
/// frontend can surface divergence / an Enforce block as a **passive status badge**
/// (never a modal); `None` when skipped (no repo configured, or `app_locked` —
/// `repo.json` is unreadable while the `AppLock` biometric launch-gate holds the
/// master key) or on a silent network error (best-effort: never nags on a flaky
/// resume). Gated on `AutoSync` being on — the frontend checks first, and the
/// backend re-checks as defense-in-depth (a compromised `WebView` invoking this
/// directly must not publish local commits when the user turned `AutoSync` off).
/// Emits `"sync-outcome"` so a mounted entry list can refresh on a fast-forward.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn background_sync<R: Runtime>(
    state: State<'_, AppState>,
    app: AppHandle<R>,
) -> Result<Option<SyncOutcome>, Error> {
    // Defense-in-depth gates (cheap, sync): first-run + app-locked + AutoSync-off.
    // The frontend also gates, but this stops a stray (or XSS-driven) invoke from
    // touching the network before the store exists, while repo.json is sealed
    // behind the launch gate, or when the user turned AutoSync off.
    if !state.store.is_repo_ready()
        || state.app_locked.load(Ordering::SeqCst)
        || !state.store.autosync()
    {
        return Ok(None);
    }
    // Bound the best-effort sync so a stalled/malicious remote can't hold
    // `write_mu` indefinitely and queue every user save/Sync behind it. The
    // private-slot + 30s deadline live in [`run_best_effort_sync`]; this is the
    // foreground (pull+push) variant.
    let store = state.store.clone();
    let result = run_best_effort_sync(|slot, cancel| async move {
        store.sync_repo(&slot, Some(cancel), None).await
    })
    .await;
    match result {
        Ok(outcome) => {
            log::info!("background-sync: {outcome:?}");
            let _ = app.emit("sync-outcome", &outcome);
            Ok(Some(outcome))
        }
        Err(e) => {
            log::warn!("background-sync failed: {e}");
            Ok(None)
        }
    }
}

/// Push the current branch to `origin`. Used by the create flow's deferred first
/// push — called after `create_store` + `complete_setup` so the remote only
/// receives the store once its identity is durable. A missing `origin` is a
/// no-op (local-only store), mirroring `pull_repo`.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn push_repo(state: State<'_, AppState>) -> Result<(), Error> {
    log::info!("push: start");
    state
        .store
        .push()
        .await
        .inspect_err(|e| log::warn!("push failed: {e}"))
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
    log::info!("resolve: {expected_remote_oid} {choice:?}");
    let store = state.store.clone();
    let expected = expected_remote_oid;
    let result =
        crate::git::run_cancellable(&state, app.clone(), move |cancel, _tx, slot| async move {
            store
                .resolve_sync_divergence(&slot, &expected, choice, Some(cancel))
                .await
        })
        .await
        .inspect_err(|e| log::warn!("resolve failed: {e}"));
    reset_lock_timer(&state, &app);
    reset_gate_idle_timer(&state, &app);
    // D3: terminal step for a deferred save-divergence — do the wipe the save
    // path skipped (no-op under Idle/Never; under Immediate it clears the
    // identity kept alive across the modal for keep-mine).
    maybe_soft_wipe(&state, &app).await;
    result
}

/// Resolve a per-entry edit/delete conflict (R026 [`WriteOutcome::EntryConflict`])
/// by the user's `choice` against the reviewed remote tip (`expected_remote_oid`).
/// Mirrors [`resolve_sync_divergence`]: resets the auto-lock timers and runs the
/// terminal Immediate-mode wipe the save path deferred so a keep-mine edit resolve
/// could reuse the cached identity. "Cancel" is client-side — the frontend reuses
/// [`discard_divergence`].
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn resolve_entry_conflict(
    state: State<'_, AppState>,
    app: AppHandle,
    name: String,
    content: Option<String>,
    expected_remote_oid: String,
    op: ExpectedKind,
    choice: EntryConflictChoice,
) -> Result<SyncResult, Error> {
    log::info!("entry-resolve: {name} {op:?} {choice:?}");
    let store = state.store.clone();
    let content_bytes = content.map(String::into_bytes);
    let result =
        crate::git::run_cancellable(&state, app.clone(), move |cancel, _tx, slot| async move {
            store
                .resolve_entry_conflict(
                    &slot,
                    &name,
                    content_bytes.as_deref(),
                    &expected_remote_oid,
                    op,
                    choice,
                    Some(cancel),
                )
                .await
        })
        .await
        .inspect_err(|e| log::warn!("entry-resolve failed: {e}"));
    reset_lock_timer(&state, &app);
    reset_gate_idle_timer(&state, &app);
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
    log::info!("discard-divergence");
    maybe_soft_wipe(&state, &app).await;
    Ok(())
}
