// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

mod crypto;
mod error;
mod git;
mod secure_storage;
mod store;

/// Re-export core functions for integration tests.
pub mod test_support {
    pub use crate::crypto::decrypt_bytes;
    pub use crate::error::AppError;
    pub use crate::store::{list_entries, parse_decrypted_content};
}

use std::path::Path;

use error::AppError;
use secure_storage::{RepoConfig, SecureStorage};
use store::{CopyResult, Entry, PullResult, SensitiveContent};

use tauri_plugin_clipboard_manager::ClipboardExt;
use zeroize::Zeroize;

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

struct AppState {
    storage: SecureStorage,
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Check if the app has been configured (identity + repo exist).
#[tauri::command]
fn is_configured(state: tauri::State<'_, AppState>) -> Result<bool, AppError> {
    Ok(state.storage.is_configured())
}

/// Full setup: validate identity, clone repo, save config.
#[tauri::command]
fn setup(
    state: tauri::State<'_, AppState>,
    repo_url: String,
    pat: Option<String>,
    identity: String,
) -> Result<(), AppError> {
    // Validate identity format
    let identity_bytes = identity.trim().as_bytes();
    if !identity.trim().starts_with("AGE-SECRET-KEY-") {
        return Err(AppError::new(
            error::ErrorCode::InvalidIdentity,
            "Identity must start with AGE-SECRET-KEY-...",
        ));
    }

    // Determine local repo path
    let config_dir = SecureStorage::default_config_dir()?;
    let repo_dir = config_dir.join("repo");

    // Clear any existing configuration
    state.storage.clear_all()?;

    // Remove existing repo directory if present
    if repo_dir.exists() {
        std::fs::remove_dir_all(&repo_dir)?;
    }

    // Save identity first (before clone, so decrypt can work)
    state.storage.save_identity(identity_bytes)?;

    // Clone the repo
    git::clone_repo(&repo_url, &repo_dir, pat.as_deref())?;

    // Save repo config
    let local_path = repo_dir.to_string_lossy().to_string();
    state
        .storage
        .save_repo_config(&repo_url, pat.as_deref(), &local_path)?;

    Ok(())
}

/// List all .age entries in the configured repository.
#[tauri::command]
fn list_entries(state: tauri::State<'_, AppState>) -> Result<Vec<Entry>, AppError> {
    let config = state.storage.load_repo_config()?;
    let repo_path = Path::new(&config.local_path);
    store::list_entries(repo_path)
}

/// Pull latest changes (fast-forward only).
#[tauri::command]
fn pull_repo(state: tauri::State<'_, AppState>) -> Result<PullResult, AppError> {
    let config = state.storage.load_repo_config()?;
    let repo_path = Path::new(&config.local_path);
    git::pull_repo(repo_path, config.pat.as_deref())
}

/// Primary operation: decrypt and copy password to clipboard.
/// Password never reaches the WebView.
#[tauri::command]
async fn copy_password(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    entry_path: String,
) -> Result<CopyResult, AppError> {
    let config = state.storage.load_repo_config()?;
    let repo_path = Path::new(&config.local_path);

    // Resolve and validate entry path
    let file_path = store::resolve_entry_path(repo_path, &entry_path)?;

    // Load identity (caller must zeroize)
    let mut identity_bytes = state.storage.load_identity()?;

    // Decrypt
    let decrypted = crypto::decrypt_file(&file_path, &identity_bytes)?;

    // Zeroize identity immediately after decryption
    identity_bytes.zeroize();

    // Parse into password + notes
    let mut entry = store::parse_decrypted_content(&decrypted)?;

    // Derive display name from entry path
    let entry_name = entry_path.trim_end_matches(".age").to_string();

    // Copy password to clipboard via Tauri plugin
    app.clipboard()
        .write_text(entry.password.to_string())
        .map_err(|e| {
            AppError::new(
                error::ErrorCode::ClipboardError,
                format!("Clipboard error: {}", e),
            )
        })?;

    // Zeroize decrypted content immediately
    entry.password.zeroize();
    entry.notes.zeroize();

    // Spawn clipboard auto-clear after 30 seconds
    let clear_handle = app.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        let _ = clear_handle.clipboard().write_text(String::new());
    });

    Ok(CopyResult {
        success: true,
        entry_name,
        cleared_after_secs: 30,
    })
}

/// Secondary operation: decrypt and return password for display.
/// Password crosses IPC — Vue component must follow strict lifecycle.
#[tauri::command]
fn show_password(
    state: tauri::State<'_, AppState>,
    entry_path: String,
) -> Result<SensitiveContent, AppError> {
    let config = state.storage.load_repo_config()?;
    let repo_path = Path::new(&config.local_path);

    // Resolve and validate entry path
    let file_path = store::resolve_entry_path(repo_path, &entry_path)?;

    // Load identity (caller must zeroize)
    let mut identity_bytes = state.storage.load_identity()?;

    // Decrypt
    let decrypted = crypto::decrypt_file(&file_path, &identity_bytes)?;

    // Zeroize identity immediately after decryption
    identity_bytes.zeroize();

    // Parse into password + notes
    let mut entry = store::parse_decrypted_content(&decrypted)?;

    let result = SensitiveContent {
        password: entry.password.to_string(),
        notes: entry.notes.to_string(),
    };

    // Zeroize the Rust-side DecryptedEntry fields
    entry.password.zeroize();
    entry.notes.zeroize();

    Ok(result)
}

/// Get the current repo config (for display in settings).
#[tauri::command]
fn get_config(state: tauri::State<'_, AppState>) -> Result<RepoConfig, AppError> {
    state.storage.load_repo_config()
}

/// Reset all configuration and local data.
#[tauri::command]
fn reset_config(state: tauri::State<'_, AppState>) -> Result<(), AppError> {
    // Remove local repo if it exists
    if let Ok(config) = state.storage.load_repo_config() {
        let repo_path = Path::new(&config.local_path);
        if repo_path.exists() {
            std::fs::remove_dir_all(repo_path)?;
        }
    }
    state.storage.clear_all()
}

// ---------------------------------------------------------------------------
// App entry point
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let config_dir =
        SecureStorage::default_config_dir().expect("Cannot determine config directory");

    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(AppState {
            storage: SecureStorage::new(config_dir),
        })
        .invoke_handler(tauri::generate_handler![
            is_configured,
            setup,
            list_entries,
            pull_repo,
            copy_password,
            show_password,
            get_config,
            reset_config,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
