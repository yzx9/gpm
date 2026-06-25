// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Clipboard-out commands and the shared write-then-auto-clear helper. The clear
//! runs in a detached task holding an `AppHandle` clone, so it survives the
//! calling page being unmounted — the guarantee both `copy_password` (decrypts a
//! stored secret) and `copy_generated_password` (copies an in-memory string from
//! the standalone generator) rely on.

use std::time::Duration;

use rustpass::Error;
use rustpass::error::ErrorCode;
use tauri::{AppHandle, Runtime};
use tauri_plugin_clipboard_manager::ClipboardExt;
use zeroize::Zeroizing;

/// How long a copied secret lingers on the clipboard before it is cleared.
pub(crate) const CLIPBOARD_CLEAR_SECS: u32 = 30;

/// Write `text` to the system clipboard, then schedule an auto-clear after
/// [`CLIPBOARD_CLEAR_SECS`]. Returns the clipboard-write result (the scheduled
/// clear is fire-and-forget).
///
/// # Errors
///
/// Returns [`ErrorCode::StoreError`] if the initial clipboard write fails.
pub(crate) fn write_and_schedule_clear<R: Runtime>(
    app: &AppHandle<R>,
    text: String,
) -> Result<(), Error> {
    app.clipboard()
        .write_text(text)
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("Clipboard error: {e}")))?;

    let clear_handle = app.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(u64::from(CLIPBOARD_CLEAR_SECS))).await;
        let _ = clear_handle.clipboard().write_text(String::new());
    });

    Ok(())
}

/// Copy an already-decrypted/generated password to the clipboard and arm the
/// [`CLIPBOARD_CLEAR_SECS`] auto-clear. Unlike
/// [`copy_password`](crate::read::copy_password) this takes the plaintext
/// directly — the standalone generator already has it in the page — and is
/// stateless: no store, no lock-timer reset. The `Zeroizing<String>` param is
/// wiped on drop.
///
/// # Errors
///
/// Returns [`ErrorCode::StoreError`] if the clipboard is unavailable.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn copy_generated_password(
    app: AppHandle,
    text: Zeroizing<String>,
) -> Result<(), Error> {
    write_and_schedule_clear(&app, (*text).clone())
}
