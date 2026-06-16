// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Secret-read commands — list, decrypt-and-copy, and decrypt-and-show. The
//! read side of the store, mirroring [`crate::write`] on the write side.

use std::time::Duration;

use rustpass::error::ErrorCode;
use rustpass::{Entry, Error};
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::AppState;
use crate::identity::reset_lock_timer;

// ---------------------------------------------------------------------------
// Tauri-IPC types (not in rustpass — these are UI-layer concerns)
// ---------------------------------------------------------------------------

/// Returned by `copy_password` — no secret data, safe for IPC.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct CopyResult {
    success: bool,
    entry_name: String,
    cleared_after_secs: u32,
}

/// Returned by `show_password` — contains secrets, strict Vue lifecycle required.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct SensitiveContent {
    password: String,
    notes: String,
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// List all .age entries in the configured repository.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn list_entries(state: State<'_, AppState>) -> Result<Vec<Entry>, Error> {
    state.store.list().await
}

/// Primary operation: decrypt and copy password to clipboard.
/// Password never reaches the `WebView`.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn copy_password(
    state: State<'_, AppState>,
    app: AppHandle,
    entry_path: String,
) -> Result<CopyResult, Error> {
    let secret = state.store.get(&entry_path).await?;

    let entry_name = entry_path.trim_end_matches(".age").to_string();

    app.clipboard()
        .write_text(secret.password().to_string())
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("Clipboard error: {e}")))?;

    // Spawn clipboard auto-clear after 30 seconds
    let clear_handle = app.clone();
    let pw = secret.password().to_string();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(30)).await;
        let _ = clear_handle.clipboard().write_text(String::new());
        drop(pw);
    });

    // Reset auto-lock timer
    reset_lock_timer(&state, &app);

    Ok(CopyResult {
        success: true,
        entry_name,
        cleared_after_secs: 30,
    })
}

/// Secondary operation: decrypt and return password for display.
/// Password crosses IPC — Vue component must follow strict lifecycle.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn show_password(
    state: State<'_, AppState>,
    entry_path: String,
) -> Result<SensitiveContent, Error> {
    let secret = state.store.get(&entry_path).await?;

    Ok(SensitiveContent {
        password: secret.password().to_string(),
        notes: secret.body().to_string(),
    })
}
