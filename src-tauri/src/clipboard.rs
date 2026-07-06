// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Clipboard-out commands and the shared write-then-auto-clear helper. The
//! clear is armed via [`crate::identity::arm_clipboard_clear`] (cancellable,
//! replaces in-flight tasks), so it survives the calling page being unmounted
//! — the guarantee both [`copy_password`](crate::read::copy_password)
//! (decrypts a stored secret) and [`copy_generated_password`] (copies an
//! in-memory string from the standalone generator) rely on. Both paths also
//! post the sticky Android notification (best-effort; no-op on desktop).

use rustpass::error::ErrorCode;
use rustpass::Error;
use tauri::{AppHandle, Runtime, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_clipboard_notify::ClipboardNotifyExt;
use zeroize::Zeroizing;

use crate::identity::{arm_clipboard_clear, disarm_clipboard_clear};
use crate::read::clipboard_clear_plan;
use crate::AppState;

/// Write `text` to the system clipboard, then arm the cancellable auto-clear
/// for the configured `clipboard_clear_secs` and post the sticky notification.
/// Returns the clipboard-write result; the armed clear + notification are
/// best-effort.
///
/// # Errors
///
/// Returns [`ErrorCode::StoreError`] if the initial clipboard write fails.
pub(crate) async fn write_and_schedule_clear<R: Runtime>(
    state: &State<'_, AppState>,
    app: &AppHandle<R>,
    text: String,
) -> Result<(), Error> {
    app.clipboard()
        .write_text(text)
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("Clipboard error: {e}")))?;

    let clear_secs = state
        .clipboard_clear_secs
        .lock()
        .map_or_else(|_| rustpass::config::DEFAULT_CLIPBOARD_CLEAR_SECS, |s| *s);
    let (spawn_clear, _cleared_after_secs) = clipboard_clear_plan(clear_secs);
    if spawn_clear {
        arm_clipboard_clear(state, app, clear_secs);
        app.clipboard_notify().post_notification(clear_secs).await;
    } else {
        // Never: abort any in-flight clear from a prior shorter setting.
        disarm_clipboard_clear(state);
    }
    Ok(())
}

/// Copy an already-decrypted/generated password to the clipboard and arm the
/// auto-clear + sticky notification. Unlike
/// [`copy_password`](crate::read::copy_password) this takes the plaintext
/// directly — the standalone generator already has it in the page — and is
/// stateless: no store, no lock-timer reset. Uses the configured
/// `clipboard_clear_secs` (same source as `copy_password`). The
/// `Zeroizing<String>` param is wiped on drop.
///
/// # Errors
///
/// Returns [`ErrorCode::StoreError`] if the clipboard is unavailable.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn copy_generated_password(
    state: State<'_, AppState>,
    app: AppHandle,
    text: Zeroizing<String>,
) -> Result<(), Error> {
    write_and_schedule_clear(&state, &app, (*text).clone()).await
}

/// Whether the app may post notifications (Android 13+ runtime permission).
/// Cheap and non-prompting. The frontend's ask-once flow calls this before
/// copying to decide whether to prompt. Always `true` on desktop (no
/// notification-permission model there).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn are_clipboard_notifications_enabled(app: AppHandle) -> Result<bool, Error> {
    Ok(app.clipboard_notify().are_enabled().await)
}

/// Request `POST_NOTIFICATIONS` at runtime (Android 13+). Shows the system
/// dialog and returns the grant state. Always `true` on desktop.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn request_clipboard_notifications_permission(
    app: AppHandle,
) -> Result<bool, Error> {
    Ok(app.clipboard_notify().request_permission().await)
}
