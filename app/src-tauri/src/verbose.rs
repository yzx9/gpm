// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! The verbose-logging deadline timer: a cancellable, generation-guarded task
//! that reverts the runtime log gate from Debug back to Info when the verbose
//! window elapses — mid-session, no restart needed. Mirrors the cancel-and-
//! respawn shape of [`crate::identity::arm_clipboard_clear`] / `arm_lock`.
//!
//! On fire the timer clears `verbose_until` (persist), lowers
//! `log::set_max_level` to Info, and emits [`REVERTED_EVENT`] so the frontend
//! can flip the toggle and post the OS notification. The persisted deadline
//! remains the durable backstop: if the process is killed before the window
//! elapses, the next launch's `clear_expired_verbose` reverts at startup (and
//! re-arms this timer if the window is still live).

use std::sync::atomic::Ordering;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_notification::NotificationExt;
use tokio::task::JoinHandle;
use tokio::time::sleep;

use crate::AppState;
use crate::app_config::now_unix;

/// The frontend event emitted when verbose auto-reverts at the deadline. The
/// frontend flips the Logs toggle to Off and posts the OS notification.
pub(crate) const REVERTED_EVENT: &str = "verbose-reverted";

/// Revert verbose to Info now: clear the deadline, lower the runtime gate, post
/// the staged OS notification, and emit [`REVERTED_EVENT`]. The notification is
/// posted from Rust (not the `WebView`) so it fires even when the app is
/// backgrounded — the notice is the whole point of the revert, and a paused
/// `WebView` would silently drop a frontend-driven one. Best-effort persist — a
/// failure leaves the deadline on disk, cleared by the next launch.
pub(crate) async fn revert_verbose<R: Runtime>(state: &AppState, app: &AppHandle<R>) {
    let _ = state.app_config.set_verbose(false).await;
    log::set_max_level(state.app_config.effective_log_filter());
    // Post the staged notification directly (backgrounding-safe). Consumed so a
    // double-fire can't post twice.
    if let Some(text) = state.app_config.take_revert_notify() {
        let _ = app
            .notification()
            .builder()
            .title(text.title)
            .body(text.body)
            .show();
    }
    // Tell the frontend to sync the toggle to Off.
    let _ = app.emit(REVERTED_EVENT, ());
}

/// (Re)arm the verbose deadline timer to fire when `verbose_until` elapses,
/// replacing any in-flight task. No-op if no deadline is set or it has already
/// passed (the startup clear handles the expired case). Mirrors
/// `arm_clipboard_clear`: abort the existing handle, bump the generation, and
/// spawn a task that self-disarms if a newer arm (or a disarm) happened while
/// it slept.
pub(crate) fn arm_verbose_timer<R: Runtime>(state: &AppState, app: &AppHandle<R>) {
    let Some(deadline) = state.app_config.get().verbose_until else {
        return;
    };
    let now = now_unix();
    if deadline <= now {
        return;
    }
    let remaining = deadline - now;

    let Ok(mut handle) = state.verbose_timer.lock() else {
        return;
    };
    if let Some(h) = handle.take() {
        h.abort();
    }
    let generation = state.verbose_generation.fetch_add(1, Ordering::SeqCst) + 1;
    let generation_cell = state.verbose_generation.clone();
    let app_handle = app.clone();

    let task: JoinHandle<()> = tokio::spawn(async move {
        sleep(Duration::from_secs(remaining)).await;
        // Stale-task guard: a fresh arm or a disarm bumped the generation.
        if generation_cell.load(Ordering::SeqCst) != generation {
            return;
        }
        let state = app_handle.state::<AppState>();
        revert_verbose(state.inner(), &app_handle).await;
    });
    *handle = Some(task);
}

/// Disarm the verbose timer (cancel any in-flight revert). Bumps the generation
/// so a sleeping task self-disarms even if `abort` races the wake. Called on a
/// manual Off toggle.
pub(crate) fn disarm_verbose_timer(state: &AppState) {
    let Ok(mut handle) = state.verbose_timer.lock() else {
        return;
    };
    if let Some(h) = handle.take() {
        h.abort();
    }
    // Bump so any task that slipped past `abort` self-disarms on wake.
    state.verbose_generation.fetch_add(1, Ordering::SeqCst);
}
