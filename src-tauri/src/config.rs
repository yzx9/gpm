// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Repository / app configuration commands — repo config display, the commit
//! author identity, and a full reset. When this grows further (import/export,
//! per-repo settings), it can graduate to a `config/` directory of submodules.

use rustpass::{CommitIdentity, Error, LockMode, RepoConfig, Store};
use tauri::{AppHandle, State};

use crate::AppState;
use crate::app_config::AppConfig;
use crate::identity::{emit_lock_state, refresh_security_cache, reset_lock_timer};

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
    emit_lock_state(&app, &state.store, false).await;
    Ok(())
}

/// Set the git commit author identity. A `null` field clears it, reverting to
/// the app default. Returns the updated repo config.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn set_commit_identity(
    state: State<'_, AppState>,
    name: Option<String>,
    email: Option<String>,
) -> Result<RepoConfig, Error> {
    state.store.set_commit_identity(name, email).await
}

/// Set the app auto-lock mode (`immediate` / `{ idle: secs }` / `never`).
/// Refreshes the `AppState` cache and re-applies the timer so the new mode takes
/// effect immediately (Immediate/Never disarm; Idle re-arms). Returns the
/// updated app config.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn set_lock_mode(
    state: State<'_, AppState>,
    app: AppHandle,
    mode: LockMode,
) -> Result<AppConfig, Error> {
    let cfg = state.app_config.set_lock_mode(mode).await?;
    refresh_security_cache(&state).await;
    // Apply the new mode to the live timer (reads the just-refreshed cache).
    reset_lock_timer(&state, &app);
    Ok(cfg)
}

/// Set the password-view auto-clear override (`null` = default, `0` = never).
/// Returns the updated app config; the UI reads the new value via `get_app_config`.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn set_view_clear_secs(
    state: State<'_, AppState>,
    secs: Option<u64>,
) -> Result<AppConfig, Error> {
    state.app_config.set_view_clear_secs(secs).await
}

/// Set the clipboard auto-clear override (`null` = default, `0` = never).
/// Refreshes the `AppState` cache so the next copy honors it. Returns the updated
/// app config.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn set_clipboard_clear_secs(
    state: State<'_, AppState>,
    secs: Option<u64>,
) -> Result<AppConfig, Error> {
    let cfg = state.app_config.set_clipboard_clear_secs(secs).await?;
    refresh_security_cache(&state).await;
    Ok(cfg)
}

/// Set the per-device autosync flag — whether each save wraps in a pull → write
/// → push (`true`, the default) or stays local until a manual Sync. Also pushes
/// the value into the `Store`'s injected cache (`autosync_write` reads it).
/// Returns the updated app config.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn set_autosync(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<AppConfig, Error> {
    let cfg = state.app_config.set_autosync(enabled).await?;
    state.store.set_autosync(enabled);
    Ok(cfg)
}

/// The default commit author identity (for UI display).
#[tauri::command]
pub(crate) async fn get_commit_identity_default() -> CommitIdentity {
    Store::commit_identity_default()
}
