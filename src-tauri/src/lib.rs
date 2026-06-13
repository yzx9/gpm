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

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustpass::error::ErrorCode;
use rustpass::ssh;
use rustpass::{Entry, Error, KeyType, Recipient, RepoConfig, Store, SyncResult};
use serde::Serialize;
use tokio::task::JoinHandle;

use tauri::{Emitter, Manager};
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

/// Returned by `list_recipients` — public key info for setup step 2.
#[derive(Debug, Clone, Serialize)]
struct RecipientInfo {
    public_key: String,
    comment: Option<String>,
    key_type: String,
}

/// Returned by `get_auth_state` — atomic auth snapshot for router guard.
#[derive(Debug, Clone, Serialize)]
struct AuthState {
    /// True if both identity and repo config exist.
    configured: bool,
    /// True if the stored identity requires a passphrase (age-encrypted or encrypted SSH).
    encrypted: bool,
    /// True if the identity cache is populated (passphrase provided).
    unlocked: bool,
}

/// Returned by `validate_identity` — identity type and encryption status.
#[derive(Debug, Clone, Serialize)]
struct IdentityInfoResult {
    key_type: String,
    encrypted: bool,
}

impl From<Recipient> for RecipientInfo {
    fn from(r: Recipient) -> Self {
        Self {
            public_key: r.public_key,
            comment: r.comment,
            key_type: match r.key_type {
                KeyType::X25519 => "x25519".to_string(),
                KeyType::SshEd25519 => "ssh_ed25519".to_string(),
                KeyType::SshRsa => "ssh_rsa".to_string(),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

/// Application state shared across all Tauri commands.
struct AppState {
    store: Arc<Store>,
    /// Auto-lock timer handle (cancel-and-respawn pattern).
    lock_timer: Mutex<Option<JoinHandle<()>>>,
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Get the authentication state as a single atomic snapshot.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn get_auth_state(state: tauri::State<'_, AppState>) -> Result<AuthState, Error> {
    Ok(AuthState {
        configured: state.store.is_configured(),
        encrypted: state.store.is_identity_encrypted().await,
        unlocked: state.store.is_unlocked(),
    })
}

/// Check if the app has been configured (identity + repo exist).
#[tauri::command]
#[allow(clippy::needless_pass_by_value, clippy::unnecessary_wraps)]
fn is_configured(state: tauri::State<'_, AppState>) -> Result<bool, Error> {
    Ok(state.store.is_configured())
}

/// Check if the repo has been cloned (step 1 done, identity may be missing).
#[tauri::command]
#[allow(clippy::needless_pass_by_value, clippy::unnecessary_wraps)]
fn is_repo_ready(state: tauri::State<'_, AppState>) -> Result<bool, Error> {
    Ok(state.store.is_repo_ready())
}

/// Step 1 of setup: clone the repo and save repo config (no identity).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn clone_repo(
    state: tauri::State<'_, AppState>,
    repo_url: String,
    pat: Option<String>,
    ssh_key: Option<String>,
    ssh_passphrase: Option<String>,
) -> Result<(), Error> {
    state
        .store
        .clone_only(
            &repo_url,
            pat.as_deref(),
            ssh_key.as_deref(),
            ssh_passphrase.as_deref(),
        )
        .await
}

/// Read recipients from the cloned repository for setup step 2.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn list_recipients(state: tauri::State<'_, AppState>) -> Result<Vec<RecipientInfo>, Error> {
    let recipients = state.store.list_recipients().await?;
    Ok(recipients.into_iter().map(RecipientInfo::from).collect())
}

/// Validate an identity and return its type and encryption status.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn validate_identity(identity: String) -> Result<IdentityInfoResult, Error> {
    let info = rustpass::recipient::validate_identity(&identity)?;
    Ok(IdentityInfoResult {
        key_type: match info.key_type {
            KeyType::X25519 => "x25519".to_string(),
            KeyType::SshEd25519 => "ssh_ed25519".to_string(),
            KeyType::SshRsa => "ssh_rsa".to_string(),
        },
        encrypted: info.encrypted,
    })
}

/// Step 2 of setup: save the age identity and complete configuration.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn complete_setup(
    state: tauri::State<'_, AppState>,
    identity: String,
    passphrase: Option<String>,
    ssh_passphrase: Option<String>, // TODO: why there are passphrase and passphrase for ssh? can we unify them?
) -> Result<(), Error> {
    state
        .store
        .save_identity(&identity, passphrase.as_deref(), ssh_passphrase.as_deref())
        .await
}

/// Full setup: validate identity, clone repo, save config.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn setup(
    state: tauri::State<'_, AppState>,
    repo_url: String,
    pat: Option<String>,
    ssh_key: Option<String>,
    ssh_passphrase: Option<String>,
    identity: String,
    identity_passphrase: Option<String>,
) -> Result<(), Error> {
    state
        .store
        .configure(
            &repo_url,
            pat.as_deref(),
            ssh_key.as_deref(),
            ssh_passphrase.as_deref(),
            &identity,
            identity_passphrase.as_deref(),
        )
        .await
}

/// Unlock a passphrase-encrypted identity (async — scrypt is slow).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn unlock(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    passphrase: String,
) -> Result<(), Error> {
    // Store::unlock is now async and handles spawn_blocking internally
    state.store.unlock(&passphrase).await?;

    // Start auto-lock timer
    reset_lock_timer(&state, &app);

    Ok(())
}

/// Lock the store: clear cached identity and cancel auto-lock timer.
#[tauri::command]
#[allow(clippy::needless_pass_by_value, clippy::unnecessary_wraps)]
fn lock(state: tauri::State<'_, AppState>) -> Result<(), Error> {
    // Cancel timer
    if let Ok(mut timer) = state.lock_timer.lock() {
        if let Some(handle) = timer.take() {
            handle.abort();
        }
    }
    state.store.lock();
    Ok(())
}

/// Set a passphrase on an existing plaintext identity.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn set_passphrase(
    state: tauri::State<'_, AppState>,
    passphrase: String,
) -> Result<(), Error> {
    state.store.set_passphrase(&passphrase).await
}

/// Change the passphrase on an encrypted identity.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn change_passphrase(
    state: tauri::State<'_, AppState>,
    old_passphrase: String,
    new_passphrase: String,
) -> Result<(), Error> {
    state
        .store
        .change_passphrase(&old_passphrase, &new_passphrase)
        .await
}

/// List all .age entries in the configured repository.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn list_entries(state: tauri::State<'_, AppState>) -> Result<Vec<Entry>, Error> {
    state.store.list().await
}

/// Pull latest changes (fast-forward only).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn pull_repo(state: tauri::State<'_, AppState>) -> Result<SyncResult, Error> {
    state.store.sync().await
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
async fn show_password(
    state: tauri::State<'_, AppState>,
    entry_path: String,
) -> Result<SensitiveContent, Error> {
    let secret = state.store.get(&entry_path).await?;

    Ok(SensitiveContent {
        password: secret.password().to_string(),
        notes: secret.body().to_string(),
    })
}

/// Get the current repo config (for display in settings).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn get_config(state: tauri::State<'_, AppState>) -> Result<RepoConfig, Error> {
    state.store.config().await
}

/// Reset all configuration and local data.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn reset_config(state: tauri::State<'_, AppState>) -> Result<(), Error> {
    // Cancel timer
    if let Ok(mut timer) = state.lock_timer.lock() {
        if let Some(handle) = timer.take() {
            handle.abort();
        }
    }
    state.store.reset().await
}

/// Generate a new ed25519 SSH keypair for setup.
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
async fn get_ssh_public_key(
    state: tauri::State<'_, AppState>,
) -> Result<SshPublicKeyResult, Error> {
    let config = state.store.config().await?;
    let private_key = config
        .ssh_key
        .ok_or_else(|| Error::new(ErrorCode::SshKeyInvalid, "No SSH key configured"))?;
    let public_key = ssh::get_public_key(&private_key)?;
    Ok(SshPublicKeyResult { public_key })
}

/// Export the stored SSH private key (secret — requires confirmation in UI).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn export_ssh_private_key(
    state: tauri::State<'_, AppState>,
) -> Result<SshPrivateKeyResult, Error> {
    let config = state.store.config().await?;
    let private_key_pem = config
        .ssh_key
        .ok_or_else(|| Error::new(ErrorCode::SshKeyInvalid, "No SSH key configured"))?;
    let private_key = ssh::export_private_key(&private_key_pem)?;
    Ok(SshPrivateKeyResult {
        private_key: private_key.to_string(),
    })
}

// ---------------------------------------------------------------------------
// Timer helpers
// ---------------------------------------------------------------------------

/// Reset the auto-lock timer (cancel-and-respawn pattern).
fn reset_lock_timer(state: &tauri::State<'_, AppState>, app: &tauri::AppHandle) {
    let Ok(mut timer) = state.lock_timer.lock() else {
        return;
    };

    // Cancel existing timer
    if let Some(handle) = timer.take() {
        handle.abort();
    }

    // Spawn new timer
    let app_handle = app.clone();
    let store = state.store.clone();

    let handle = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(
            rustpass::store::DEFAULT_LOCK_TIMEOUT_SECS,
        ))
        .await;

        // Lock the real store (clears cached identity + passphrase)
        store.lock();

        // Emit lock event so frontend can redirect
        let _ = app_handle.emit("identity-locked", ());
    });

    *timer = Some(handle);
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
        .plugin(gpm_plugin_safe_area::init())
        .setup(|app| {
            let config_dir = app
                .path()
                .app_config_dir()
                .expect("Cannot determine app config directory");
            app.manage(AppState {
                store: Arc::new(Store::new(config_dir)),
                lock_timer: Mutex::new(None),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_auth_state,
            is_configured,
            is_repo_ready,
            clone_repo,
            list_recipients,
            validate_identity,
            complete_setup,
            setup,
            unlock,
            lock,
            set_passphrase,
            change_passphrase,
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
