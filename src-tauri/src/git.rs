// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Git operation progress + cancellation bridge.
//!
//! `rustpass` reports transfer progress over a synchronous
//! `std::sync::mpsc` sender (safe to call from git2's C callbacks on the
//! blocking thread) and polls an `Arc<AtomicBool>` cancel token. This module
//! owns the channel receiver: a `spawn_blocking` drain task forwards each
//! [`rustpass::GitProgress`] to the `WebView` as a `"git-progress"` event, and the
//! [`cancel_git`] command flips the active token to abort an in-flight clone or
//! pull.

use std::sync::atomic::Ordering;

use rustpass::{Error, GitProgress, ProgressSender};
use tauri::{Emitter, Runtime, State};

use crate::AppState;

/// Frontend-facing subset of [`GitProgress`]: just the fields the progress bar
/// needs. Object/delta indexing detail stays backend-internal.
#[derive(Debug, Clone, serde::Serialize)]
struct GitProgressEvent {
    total_objects: usize,
    received_objects: usize,
    received_bytes: usize,
    message: Option<String>,
}

impl From<&GitProgress> for GitProgressEvent {
    fn from(p: &GitProgress) -> Self {
        Self {
            total_objects: p.total_objects,
            received_objects: p.received_objects,
            received_bytes: p.received_bytes,
            message: p.message.clone(),
        }
    }
}

/// Create a progress channel pair and spawn a blocking drain task that emits
/// each [`GitProgress`] as a `"git-progress"` event.
///
/// The returned sender is handed to the Store method; when the git operation
/// finishes and drops it, the channel closes and the drain task exits — so
/// awaiting the join handle flushes the final events before the command returns.
pub(crate) fn spawn_progress_drain<R: Runtime>(
    app: tauri::AppHandle<R>,
) -> (ProgressSender, tauri::async_runtime::JoinHandle<()>) {
    let (tx, rx) = std::sync::mpsc::channel::<GitProgress>();
    let join = tauri::async_runtime::spawn_blocking(move || {
        while let Ok(p) = rx.recv() {
            let _ = app.emit("git-progress", GitProgressEvent::from(&p));
        }
    });
    (tx, join)
}

/// A fresh, unset cancel token for an upcoming clone/pull.
pub(crate) fn fresh_cancel_token() -> rustpass::CancelToken {
    std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false))
}

/// Publish `token` as the active cancel token so [`cancel_git`] can abort the
/// in-flight operation. Stores a clone — the command keeps its own to pass into
/// the Store method (both share the same `AtomicBool`).
pub(crate) fn arm_cancel(state: &State<'_, AppState>, token: rustpass::CancelToken) {
    *state
        .active_cancel_token
        .lock()
        .expect("active_cancel_token lock poisoned") = Some(token);
}

/// Clear the active cancel token once the operation has settled (success,
/// failure, or cancel) — no-op if none was armed.
pub(crate) fn disarm_cancel(state: &State<'_, AppState>) {
    *state
        .active_cancel_token
        .lock()
        .expect("active_cancel_token lock poisoned") = None;
}

/// Run a cancellable, progress-reporting git operation.
///
/// Arms a fresh cancel token (so [`cancel_git`] can abort it), spawns a progress
/// drain that forwards `"git-progress"` events, runs `op`, then clears the token
/// and flushes the drain's final events before returning. Owns the arm → op →
/// disarm → drain-await ordering in one place so the cancellable commands stay
/// in sync.
pub(crate) async fn run_cancellable<R, F, Fut, T>(
    state: &State<'_, AppState>,
    app: tauri::AppHandle<R>,
    op: F,
) -> Result<T, Error>
where
    R: Runtime,
    F: FnOnce(rustpass::CancelToken, ProgressSender) -> Fut,
    Fut: Future<Output = Result<T, Error>>,
{
    let cancel = fresh_cancel_token();
    arm_cancel(state, cancel.clone());
    let (tx, drain) = spawn_progress_drain(app);
    let result = op(cancel, tx).await;
    disarm_cancel(state);
    let _ = drain.await;
    result
}

/// Cancel the in-flight clone/pull, if any. Flips the active cancel token so
/// git2's `transfer_progress` callback returns `false` on its next tick and
/// libgit2 aborts the transfer (the Store method then maps it to `CANCELLED`).
#[tauri::command]
#[allow(clippy::needless_pass_by_value, clippy::unnecessary_wraps)]
pub(crate) fn cancel_git(state: State<'_, AppState>) -> Result<(), Error> {
    if let Some(token) = state
        .active_cancel_token
        .lock()
        .expect("active_cancel_token lock poisoned")
        .take()
    {
        token.store(true, Ordering::Relaxed);
    }
    Ok(())
}
