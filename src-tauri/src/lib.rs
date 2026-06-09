// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! GPM — age-only gopass password manager client built with Tauri v2.

#![warn(
    trivial_casts,
    trivial_numeric_casts,
    missing_copy_implementations,
    missing_debug_implementations,
    missing_docs,
    unsafe_code,
    unstable_features,
    unused_import_braces,
    unused_qualifications,
    clippy::dbg_macro,
    clippy::indexing_slicing,
    clippy::pedantic
)]

use rustpass::error::ErrorCode;
use rustpass::ssh;
use rustpass::{Entry, Error, RepoConfig, Store, SyncResult};
use serde::Serialize;

use tauri::Manager;
use tauri_plugin_clipboard_manager::ClipboardExt;

// ---------------------------------------------------------------------------
// Tauri-IPC types (not in rustpass — these are UI-layer concerns)
// ---------------------------------------------------------------------------

/// Returned by `copy_password` — no secret data, safe for IPC.
#[derive(Debug, Clone, Serialize)]
struct CopyResult {
    success: bool,
    entry_name: String,
    cleared_after_secs: u32,
}

/// Returned by `show_password` — contains secrets, strict Vue lifecycle required.
#[derive(Debug, Clone, Serialize)]
struct SensitiveContent {
    password: String,
    notes: String,
}

/// Returned by `generate_ssh_key` — contains both keys for setup form.
#[derive(Debug, Clone, Serialize)]
struct SshKeyPairResult {
    public_key: String,
    private_key: String,
}

/// Returned by `get_ssh_public_key` — public key only, safe to display.
#[derive(Debug, Clone, Serialize)]
struct SshPublicKeyResult {
    public_key: String,
}

/// Returned by `export_ssh_private_key` — secret, strict Vue lifecycle required.
#[derive(Debug, Clone, Serialize)]
struct SshPrivateKeyResult {
    private_key: String,
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

/// Application state shared across all Tauri commands.
struct AppState {
    store: Store,
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Check if the app has been configured (identity + repo exist).
#[tauri::command]
#[allow(clippy::needless_pass_by_value, clippy::unnecessary_wraps)]
fn is_configured(state: tauri::State<'_, AppState>) -> Result<bool, Error> {
    Ok(state.store.is_configured())
}

/// Full setup: validate identity, clone repo, save config.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn setup(
    state: tauri::State<'_, AppState>,
    repo_url: String,
    pat: Option<String>,
    ssh_key: Option<String>,
    ssh_passphrase: Option<String>,
    identity: String,
) -> Result<(), Error> {
    state.store.configure(
        &repo_url,
        pat.as_deref(),
        ssh_key.as_deref(),
        ssh_passphrase.as_deref(),
        &identity,
    )
}

/// List all .age entries in the configured repository.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn list_entries(state: tauri::State<'_, AppState>) -> Result<Vec<Entry>, Error> {
    state.store.list()
}

/// Pull latest changes (fast-forward only).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn pull_repo(state: tauri::State<'_, AppState>) -> Result<SyncResult, Error> {
    state.store.sync()
}

/// Primary operation: decrypt and copy password to clipboard.
/// Password never reaches the `WebView`.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn copy_password(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    entry_path: String,
) -> Result<CopyResult, Error> {
    let secret = state.store.get(&entry_path)?;

    // Derive display name from entry path
    let entry_name = entry_path.trim_end_matches(".age").to_string();

    // Copy password to clipboard via Tauri plugin
    app.clipboard()
        .write_text(secret.password().to_string())
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("Clipboard error: {e}")))?;

    // Spawn clipboard auto-clear after 30 seconds
    let clear_handle = app.clone();
    let pw = secret.password().to_string();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        let _ = clear_handle.clipboard().write_text(String::new());
        drop(pw);
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
#[allow(clippy::needless_pass_by_value)]
fn show_password(
    state: tauri::State<'_, AppState>,
    entry_path: String,
) -> Result<SensitiveContent, Error> {
    let secret = state.store.get(&entry_path)?;

    let result = SensitiveContent {
        password: secret.password().to_string(),
        notes: secret.body().to_string(),
    };

    Ok(result)
}

/// Get the current repo config (for display in settings).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn get_config(state: tauri::State<'_, AppState>) -> Result<RepoConfig, Error> {
    state.store.config()
}

/// Reset all configuration and local data.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn reset_config(state: tauri::State<'_, AppState>) -> Result<(), Error> {
    state.store.reset()
}

/// Generate a new ed25519 SSH keypair for setup.
///
/// Private key crosses IPC — equivalent security to pasting a key.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn generate_ssh_key(passphrase: Option<String>) -> Result<SshKeyPairResult, Error> {
    let pair = ssh::generate_keypair(passphrase.as_deref())?;
    Ok(SshKeyPairResult {
        public_key: pair.public_key,
        private_key: pair.private_key.to_string(),
    })
}

/// Get the public key derived from the stored SSH private key.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn get_ssh_public_key(state: tauri::State<'_, AppState>) -> Result<SshPublicKeyResult, Error> {
    let config = state.store.config()?;
    let private_key = config
        .ssh_key
        .ok_or_else(|| Error::new(ErrorCode::SshKeyInvalid, "No SSH key configured"))?;
    let public_key = ssh::get_public_key(&private_key)?;
    Ok(SshPublicKeyResult { public_key })
}

/// Export the stored SSH private key (secret — requires confirmation in UI).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn export_ssh_private_key(state: tauri::State<'_, AppState>) -> Result<SshPrivateKeyResult, Error> {
    let config = state.store.config()?;
    let private_key_pem = config
        .ssh_key
        .ok_or_else(|| Error::new(ErrorCode::SshKeyInvalid, "No SSH key configured"))?;
    let private_key = ssh::export_private_key(&private_key_pem)?;
    Ok(SshPrivateKeyResult {
        private_key: private_key.to_string(),
    })
}

// ---------------------------------------------------------------------------
// App entry point
// ---------------------------------------------------------------------------

/// Application entry point.
///
/// # Panics
///
/// Panics if the config directory cannot be determined or if the Tauri
/// runtime fails to start.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let config_dir = app
                .path()
                .app_config_dir()
                .expect("Cannot determine app config directory");
            app.manage(AppState {
                store: Store::new(config_dir),
            });
            Ok(())
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
            generate_ssh_key,
            get_ssh_public_key,
            export_ssh_private_key,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
