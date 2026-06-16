// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Repository / app configuration commands — repo config display and a full
//! reset. Only two today; expected to grow (import/export, per-repo settings).
//! When it does, this file can graduate to a `config/` directory of submodules.

use rustpass::{Error, RepoConfig};
use tauri::{AppHandle, State};

use crate::AppState;
use crate::identity::emit_lock_state;

/// Get the current repo config (for display in settings).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn get_config(state: State<'_, AppState>) -> Result<RepoConfig, Error> {
    state.store.config().await
}

/// Reset all configuration and local data.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn reset_config(state: State<'_, AppState>, app: AppHandle) -> Result<(), Error> {
    // Cancel timer
    if let Ok(mut timer) = state.lock_timer.lock()
        && let Some(handle) = timer.take()
    {
        handle.abort();
    }
    state.store.reset().await?;
    // After a reset there is no identity, so the app is no longer locked — emit
    // the real state so any open unlock overlay closes.
    emit_lock_state(&app, &state.store).await;
    Ok(())
}
