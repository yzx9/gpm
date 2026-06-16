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

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustpass::error::ErrorCode;
use rustpass::ssh;
use rustpass::{
    AuthenticityConfig, CommitSigInfo, CommitSigStatus, Entry, Error, IdentityInfo, KeyType,
    Recipient, RepoConfig, Store, SyncResult, TrustedKey, VerifyMode,
};
use serde::Serialize;
use tokio::task::JoinHandle;
use zeroize::Zeroizing;

use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_biometric_keystore::KeystoreExt;
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_file_picker::FilePickerExt;

mod write;

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
    /// Identity type string (`x25519`, `ssh_ed25519`, `ssh_rsa`, `age_encrypted`,
    /// `post_quantum`, `unknown`) — lets the UI branch on whether the identity
    /// is an SSH key.
    identity_type: String,
}

/// Returned by `validate_identity` — identity type and encryption status.
#[derive(Debug, Clone, Serialize)]
struct IdentityInfoResult {
    key_type: String,
    encrypted: bool,
}

/// Returned by `pick_identity_file` — identity metadata only. The file contents
/// are held in backend state and never sent to the `WebView`.
#[derive(Debug, Clone, Serialize)]
struct PickedIdentityResult {
    key_type: String,
    encrypted: bool,
    filename: Option<String>,
    /// Derived public key (recipient). `Some` only when the identity is already
    /// usable (unencrypted); `None` until a passphrase is verified.
    recipient: Option<String>,
}

/// Returned by `verify_picked_identity` — the public key now that the encrypted
/// identity has been unlocked.
#[derive(Debug, Clone, Serialize)]
struct VerifiedIdentityResult {
    recipient: String,
}

/// Returned by `get_authenticity_state` — the cached snapshot for the
/// entry-list indicator badge (mode + current HEAD verification status).
#[derive(Debug, Clone, Serialize)]
struct AuthenticityState {
    mode: VerifyMode,
    head_status: CommitSigStatus,
}

/// App-local error for the biometric commands.
///
/// Serializes to `{ code, message }` — the same shape as `rustpass::Error` —
/// so the frontend can destructure both uniformly. Carries the Kotlin
/// `BIOMETRIC_*` codes (via [`From<KeystoreError>`]) and maps
/// `rustpass::Error` (via [`From<Error>`]) so a stale stored passphrase's
/// `WRONG_PASSPHRASE` reaches the frontend. `rustpass::ErrorCode` is not
/// touched; this type lives entirely in the app layer.
#[derive(Debug, Clone, Serialize)]
struct BiometricError {
    code: String,
    message: String,
}

impl From<Error> for BiometricError {
    fn from(e: Error) -> Self {
        Self {
            code: e.code,
            message: e.message,
        }
    }
}

impl From<tauri_plugin_biometric_keystore::KeystoreError> for BiometricError {
    fn from(e: tauri_plugin_biometric_keystore::KeystoreError) -> Self {
        Self {
            code: e.code,
            message: e.message,
        }
    }
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
                KeyType::PostQuantum => "post_quantum".to_string(),
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
    /// Monotonic generation tag for the auto-lock timer. Bumped on every (re)arm; the spawned
    /// task captures its generation and self-disarms if a newer arm happened while it slept.
    /// Kills the spurious re-lock race where a stale timer wakes right after a fresh unlock
    /// — the modal auto-prompts, so such a re-lock would visibly re-show the overlay.
    lock_generation: Arc<AtomicU64>,
    /// Identity picked via the file picker, awaiting its passphrase before
    /// `complete_setup_from_file` saves it. Held only in memory (`Zeroizing` on
    /// drop); never persisted.
    pending_identity: Mutex<Option<PendingIdentity>>,
    /// A write that collided with a newer remote copy, awaiting the user's
    /// resolution. Wrapped in `Arc` so the auto-lock timer closure can clear it.
    /// See `write::PendingWrite` / `write::clear_pending`.
    pending_write: Arc<Mutex<Option<write::PendingWrite>>>,
}

/// A file-picked identity awaiting save.
///
/// Does not derive `Debug` — `identity` is secret. The frontend only ever sees
/// metadata (via [`PickedIdentityResult`] / [`VerifiedIdentityResult`]).
struct PendingIdentity {
    /// The usable identity text `Store::save_identity` receives. For an
    /// age-encrypted upload this is replaced with the decrypted bare key after
    /// verification.
    identity: Zeroizing<String>,
    /// Type + encryption status of the *current* identity (updated to
    /// unencrypted after an age-encrypted identity is decrypted at verify time).
    info: IdentityInfo,
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Get the authentication state as a single atomic snapshot.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn get_auth_state(state: State<'_, AppState>) -> Result<AuthState, Error> {
    let itype = state.store.identity_type().await;
    Ok(AuthState {
        configured: state.store.is_configured(),
        encrypted: state.store.is_identity_encrypted().await,
        unlocked: state.store.is_unlocked(),
        identity_type: identity_type_string(itype),
    })
}

/// Map an [`IdentityType`](rustpass::identity::IdentityType) to a stable string
/// for IPC, matching the `key_type` values returned by [`validate_identity`].
fn identity_type_string(itype: rustpass::identity::IdentityType) -> String {
    use rustpass::identity::IdentityType;
    match itype {
        IdentityType::X25519 => "x25519",
        IdentityType::SshEd25519 => "ssh_ed25519",
        IdentityType::SshRsa => "ssh_rsa",
        IdentityType::AgeEncrypted => "age_encrypted",
        IdentityType::PostQuantum => "post_quantum",
        IdentityType::Unknown => "unknown",
    }
    .to_string()
}

/// Map a [`KeyType`] to the stable IPC string used by `validate_identity` and
/// `pick_identity_file`.
fn key_type_string(key_type: KeyType) -> &'static str {
    match key_type {
        KeyType::X25519 => "x25519",
        KeyType::SshEd25519 => "ssh_ed25519",
        KeyType::SshRsa => "ssh_rsa",
        KeyType::PostQuantum => "post_quantum",
    }
}

/// Map a [`tauri_plugin_file_picker::FilePickerError`] into the app's IPC error
/// type, turning a Kotlin `CANCELLED` into [`ErrorCode::Cancelled`].
fn map_file_picker_error(e: tauri_plugin_file_picker::FilePickerError) -> Error {
    let code = match e.code.as_str() {
        "CANCELLED" => ErrorCode::Cancelled,
        _ => ErrorCode::InvalidIdentity,
    };
    Error::new(code, e.message)
}

/// Check if the app has been configured (identity + repo exist).
#[tauri::command]
#[allow(clippy::needless_pass_by_value, clippy::unnecessary_wraps)]
fn is_configured(state: State<'_, AppState>) -> Result<bool, Error> {
    Ok(state.store.is_configured())
}

/// Check if the repo has been cloned (step 1 done, identity may be missing).
#[tauri::command]
#[allow(clippy::needless_pass_by_value, clippy::unnecessary_wraps)]
fn is_repo_ready(state: State<'_, AppState>) -> Result<bool, Error> {
    Ok(state.store.is_repo_ready())
}

/// Step 1 of setup: clone the repo and save repo config (no identity).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn clone_repo(
    state: State<'_, AppState>,
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
async fn list_recipients(state: State<'_, AppState>) -> Result<Vec<RecipientInfo>, Error> {
    let recipients = state.store.list_recipients().await?;
    Ok(recipients.into_iter().map(RecipientInfo::from).collect())
}

/// Validate an identity and return its type and encryption status.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn validate_identity(identity: String) -> Result<IdentityInfoResult, Error> {
    let info = rustpass::recipient::validate_identity(&identity)?;
    Ok(IdentityInfoResult {
        key_type: key_type_string(info.key_type).to_string(),
        encrypted: info.encrypted,
    })
}

/// Step 2 of setup: save the age identity and complete configuration.
///
/// The `passphrase` is used based on identity type: for x25519 keys it
/// optionally encrypts the identity at rest; for SSH keys it decrypts the
/// private key for recipient derivation (SSH keys are never re-encrypted by
/// gpm — they rely on their own passphrase protection, matching age's design).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn complete_setup(
    state: State<'_, AppState>,
    app: AppHandle,
    identity: String,
    passphrase: Option<String>,
) -> Result<(), Error> {
    state
        .store
        .save_identity(&identity, passphrase.as_deref())
        .await?;
    // Setup may leave an encrypted identity locked (the passphrase isn't cached);
    // emit the real state so the frontend shows the unlock overlay if needed.
    emit_lock_state(&app, &state.store).await;
    Ok(())
}

/// Pick an identity file via the native picker, classify it, and hold it in
/// backend state. Returns metadata + the public key when already usable
/// (unencrypted). Encrypted identities return `recipient: None` and must be
/// unlocked via [`verify_picked_identity`] before they can be used. The file
/// contents never reach the `WebView`.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn pick_identity_file(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<PickedIdentityResult, Error> {
    use rustpass::identity::{IdentityType, classify_identity};

    let picked = app
        .file_picker()
        .pick()
        .await
        .map_err(map_file_picker_error)?;

    let text = std::str::from_utf8(&picked.bytes).map_err(|_| {
        Error::new(
            ErrorCode::InvalidIdentity,
            "Identity file is not valid UTF-8",
        )
    })?;

    let (info, recipient) = match classify_identity(&picked.bytes) {
        IdentityType::X25519 | IdentityType::SshEd25519 | IdentityType::SshRsa => {
            let info = rustpass::recipient::validate_identity(text)?;
            // Derive the public key immediately when no passphrase is needed.
            let recipient = if info.encrypted {
                None
            } else {
                Some(rustpass::recipient::identity_to_recipient(text, None)?)
            };
            (info, recipient)
        }
        IdentityType::AgeEncrypted => {
            // A passphrase-encrypted x25519 identity (e.g. encrypted with age).
            // Cannot be used until unlocked.
            (
                IdentityInfo {
                    key_type: KeyType::X25519,
                    encrypted: true,
                },
                None,
            )
        }
        IdentityType::PostQuantum => {
            return Err(Error::new(
                ErrorCode::PostQuantumNotSupported,
                "Post-quantum (ML-KEM-768 / X-Wing) age keys aren't supported yet",
            ));
        }
        IdentityType::Unknown => {
            return Err(Error::new(
                ErrorCode::InvalidIdentity,
                "File is not a recognized age or SSH identity",
            ));
        }
    };

    let result = PickedIdentityResult {
        key_type: key_type_string(info.key_type).to_string(),
        encrypted: info.encrypted,
        filename: picked.filename.clone(),
        recipient: recipient.clone(),
    };

    // Hold the identity text (still encrypted for age-encrypted / SSH); drop any
    // previously picked identity (Zeroizing on drop).
    {
        let mut guard = state
            .pending_identity
            .lock()
            .expect("pending_identity lock poisoned");
        *guard = Some(PendingIdentity {
            identity: Zeroizing::new(text.to_string()),
            info,
        });
    }
    Ok(result)
}

/// Verify the passphrase for a picked encrypted identity and derive its public
/// key. On success the pending identity becomes usable (an age-encrypted file is
/// decrypted to its bare key). On failure the pending identity is **dropped**
/// (the file is abandoned) and `WRONG_PASSPHRASE` is returned.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn verify_picked_identity(
    state: State<'_, AppState>,
    passphrase: String,
) -> Result<VerifiedIdentityResult, Error> {
    // Take the pending identity; it is only re-stored on success, so any error
    // (incl. a wrong passphrase) abandons the file.
    let pending = {
        let mut guard = state
            .pending_identity
            .lock()
            .expect("pending_identity lock poisoned");
        guard.take()
    }
    .ok_or_else(|| Error::new(ErrorCode::NoIdentity, "No identity file selected"))?;

    let mut pending = pending;

    let recipient = match (pending.info.key_type, pending.info.encrypted) {
        // Encrypted SSH: validate the passphrase, then derive the recipient.
        (KeyType::SshEd25519 | KeyType::SshRsa, true) => {
            let pw = passphrase.clone();
            let bytes: Vec<u8> = pending.identity.as_bytes().to_vec();
            tauri::async_runtime::spawn_blocking(move || {
                rustpass::crypto::validate_ssh_key_passphrase(&bytes, &pw)
            })
            .await
            .map_err(|e| Error::new(ErrorCode::StoreError, e.to_string()))??;
            rustpass::recipient::identity_to_recipient(&pending.identity, Some(&passphrase))?
        }
        // Age-encrypted x25519: decrypt to the bare key, then derive.
        (KeyType::X25519, true) => {
            let pw = passphrase.clone();
            let enc: Vec<u8> = pending.identity.as_bytes().to_vec();
            let bare = tauri::async_runtime::spawn_blocking(move || {
                rustpass::crypto::decrypt_identity(&pw, &enc)
            })
            .await
            .map_err(|e| Error::new(ErrorCode::StoreError, e.to_string()))??;
            let bare_str = std::str::from_utf8(&bare).map_err(|_| {
                Error::new(
                    ErrorCode::InvalidIdentity,
                    "Decrypted identity is not valid UTF-8",
                )
            })?;
            let bare_info = rustpass::recipient::validate_identity(bare_str)?;
            let recipient = rustpass::recipient::identity_to_recipient(bare_str, None)?;
            // The pending identity is now the decrypted bare key.
            pending.identity = Zeroizing::new(bare_str.to_string());
            pending.info = bare_info;
            recipient
        }
        _ => {
            return Err(Error::new(
                ErrorCode::IdentityNotEncrypted,
                "Identity is not encrypted — nothing to verify",
            ));
        }
    };

    // Re-store the now-usable identity.
    {
        let mut guard = state
            .pending_identity
            .lock()
            .expect("pending_identity lock poisoned");
        *guard = Some(pending);
    }
    Ok(VerifiedIdentityResult { recipient })
}

/// Step 2 (file path): save the previously picked (and, if encrypted, verified)
/// identity. The pending identity is already usable at this point.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn complete_setup_from_file(
    state: State<'_, AppState>,
    app: AppHandle,
    passphrase: Option<String>,
) -> Result<(), Error> {
    let pending = state
        .pending_identity
        .lock()
        .expect("pending_identity lock poisoned")
        .take()
        .ok_or_else(|| Error::new(ErrorCode::NoIdentity, "No identity file selected"))?;

    state
        .store
        .save_identity(&pending.identity, passphrase.as_deref())
        .await?;
    // See [`complete_setup`]: emit the real post-setup lock state.
    emit_lock_state(&app, &state.store).await;
    Ok(())
}

/// Drop any identity held from a prior `pick_identity_file` (e.g. on back /
/// unmount), so it cannot be saved later by accident.
#[tauri::command]
#[allow(clippy::needless_pass_by_value, clippy::unnecessary_wraps)]
fn clear_pending_identity(state: State<'_, AppState>) -> Result<(), Error> {
    let _ = state
        .pending_identity
        .lock()
        .expect("pending_identity lock poisoned")
        .take();
    Ok(())
}

/// Full setup: validate identity, clone repo, save config.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn setup(
    state: State<'_, AppState>,
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
    state: State<'_, AppState>,
    app: AppHandle,
    passphrase: String,
) -> Result<(), Error> {
    unlock_and_arm(&state, &app, &passphrase).await
}

// ---------------------------------------------------------------------------
// Biometric unlock commands
// ---------------------------------------------------------------------------

/// Whether biometric-gated storage is usable on this device (API 30+ with a
/// STRONG biometric enrolled). `false` on desktop and Android <11.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn is_biometric_available(app: AppHandle) -> Result<bool, BiometricError> {
    Ok(app.keystore().is_available()?)
}

/// Whether a passphrase is sealed in the Keystore — the single source of
/// truth for "biometric is enabled" (no flag file). `false` on desktop.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn is_biometric_unlock_enabled(app: AppHandle) -> Result<bool, BiometricError> {
    Ok(app.keystore().has_stored()?)
}

/// Enable biometric unlock: validate the passphrase (D4), then seal it behind
/// a biometric prompt (D2 — encrypt also needs auth for a
/// `setUserAuthenticationRequired` key).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn enable_biometric_unlock(
    state: State<'_, AppState>,
    app: AppHandle,
    passphrase: String,
) -> Result<(), BiometricError> {
    // D4: reject a wrong passphrase before sealing it (age or SSH).
    state.store.validate_passphrase(&passphrase).await?;
    // D2: the Kotlin `store` shows a CryptoObject ENCRYPT biometric prompt.
    app.keystore().store(&passphrase).await?;
    Ok(())
}

/// Unlock via biometrics: retrieve the sealed passphrase and run it through
/// the same `unlock_and_arm` path as the password UI. If the stored passphrase
/// is stale (age path returns `WRONG_PASSPHRASE`), self-heal by deleting it so
/// it stops auto-prompting and the form is revealed for re-enabling.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn biometric_unlock(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), BiometricError> {
    // Flows Kotlin → Rust (never the WebView); wipe as soon as it's used.
    let passphrase = Zeroizing::new(app.keystore().retrieve().await?);

    if let Err(e) = unlock_and_arm(&state, &app, &passphrase).await {
        if e.code == "WRONG_PASSPHRASE" {
            // Stale sealed passphrase — clear it so the page reveals the form.
            let _ = app.keystore().delete();
        }
        return Err(BiometricError::from(e));
    }
    Ok(())
}

/// Disable biometric unlock: best-effort delete the sealed passphrase + key.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn disable_biometric_unlock(app: AppHandle) -> Result<(), BiometricError> {
    app.keystore().delete()?;
    Ok(())
}

/// Lock the store: clear cached identity and cancel auto-lock timer.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn lock(state: State<'_, AppState>, app: AppHandle) -> Result<(), Error> {
    // Cancel timer
    if let Ok(mut timer) = state.lock_timer.lock()
        && let Some(handle) = timer.take()
    {
        handle.abort();
    }
    // Disarm any racing in-flight timer task (see [`reset_lock_timer`]).
    state.lock_generation.fetch_add(1, Ordering::SeqCst);
    state.store.lock();
    // A conflict left pending would be undecryptable behind the wiped identity.
    write::clear_pending(&state.pending_write);
    // Emit the current lock state — same path the auto-lock timer takes.
    emit_lock_state(&app, &state.store).await;
    Ok(())
}

/// Set a passphrase on an existing plaintext identity.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn set_passphrase(
    state: State<'_, AppState>,
    app: AppHandle,
    passphrase: String,
) -> Result<(), Error> {
    state.store.set_passphrase(&passphrase).await?;
    // The sealed biometric passphrase (if any) is now stale — invalidate it.
    let _ = app.keystore().delete();
    // Setting a passphrase locks the store (forces re-auth with the new
    // passphrase); emit the real state so the frontend shows the overlay.
    emit_lock_state(&app, &state.store).await;
    Ok(())
}

/// Change the passphrase on an encrypted identity.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn change_passphrase(
    state: State<'_, AppState>,
    app: AppHandle,
    old_passphrase: String,
    new_passphrase: String,
) -> Result<(), Error> {
    state
        .store
        .change_passphrase(&old_passphrase, &new_passphrase)
        .await?;
    // The sealed biometric passphrase (if any) is now stale — invalidate it.
    let _ = app.keystore().delete();
    // Changing the passphrase locks the store; emit the real state.
    emit_lock_state(&app, &state.store).await;
    Ok(())
}

/// List all .age entries in the configured repository.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn list_entries(state: State<'_, AppState>) -> Result<Vec<Entry>, Error> {
    state.store.list().await
}

/// Pull latest changes (fast-forward only).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn pull_repo(state: State<'_, AppState>) -> Result<SyncResult, Error> {
    state.store.sync().await
}

/// Primary operation: decrypt and copy password to clipboard.
/// Password never reaches the `WebView`.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn copy_password(
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
async fn show_password(
    state: State<'_, AppState>,
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
async fn get_config(state: State<'_, AppState>) -> Result<RepoConfig, Error> {
    state.store.config().await
}

/// Reset all configuration and local data.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn reset_config(state: State<'_, AppState>, app: AppHandle) -> Result<(), Error> {
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
async fn get_ssh_public_key(state: State<'_, AppState>) -> Result<SshPublicKeyResult, Error> {
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
async fn export_ssh_private_key(state: State<'_, AppState>) -> Result<SshPrivateKeyResult, Error> {
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
// Repository authenticity commands
// ---------------------------------------------------------------------------

/// Cached authenticity snapshot for the entry-list indicator badge.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn get_authenticity_state(state: State<'_, AppState>) -> Result<AuthenticityState, Error> {
    let mode = state
        .store
        .authenticity_config()
        .await
        .map_or(VerifyMode::Off, |c| c.mode);
    // If HEAD status can't be computed (e.g. repo mid-clone), surface Unknown.
    let head_status = state
        .store
        .head_signature_status()
        .await
        .unwrap_or(CommitSigStatus::Unknown);
    Ok(AuthenticityState { mode, head_status })
}

/// Set the verification mode (Off / Audit / Enforce). Enforce is refused
/// until at least one trusted signing key is recorded.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn set_verification_mode(
    state: State<'_, AppState>,
    mode: VerifyMode,
) -> Result<VerifyMode, Error> {
    state.store.set_verification_mode(mode).await
}

/// Read the persisted authenticity config (no secrets — public trust anchors).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn get_authenticity_config(state: State<'_, AppState>) -> Result<AuthenticityConfig, Error> {
    state.store.authenticity_config().await
}

/// Add a trusted signing public key (validated + deduped by fingerprint).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn add_trusted_key(
    state: State<'_, AppState>,
    public_key: String,
    label: String,
) -> Result<TrustedKey, Error> {
    state.store.add_trusted_key(&public_key, &label).await
}

/// Remove a trusted signing key by fingerprint (last-key removal in Enforce
/// auto-downgrades to Audit).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn remove_trusted_key(state: State<'_, AppState>, fingerprint: String) -> Result<(), Error> {
    state.store.remove_trusted_key(&fingerprint).await
}

/// Trust HEAD's SSH-signature signer ("trust this signer" TOFU). Errors if HEAD
/// is unsigned or not SSH-signed.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn trust_head_signer(state: State<'_, AppState>, label: String) -> Result<TrustedKey, Error> {
    let public_key = state.store.head_signer_public_key().await?.ok_or_else(|| {
        Error::new(
            ErrorCode::SshKeyInvalid,
            "HEAD is not signed by an SSH key — nothing to trust.",
        )
    })?;
    state.store.add_trusted_key(&public_key, &label).await
}

/// Trust the SSH-signature signer of a specific commit ("trust this signer"
/// TOFU from the history detail view).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn trust_commit_signer(
    state: State<'_, AppState>,
    commit: String,
    label: String,
) -> Result<TrustedKey, Error> {
    state.store.trust_commit_signer(&commit, &label).await
}

/// Dismiss a specific commit's issue (per-commit + per-status ignore).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn ignore_commit_issue(state: State<'_, AppState>, commit: String) -> Result<(), Error> {
    state.store.ignore_commit_issue(&commit).await
}

/// List recent commits with per-commit signature status (the `/history` screen).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn list_commit_signatures(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<CommitSigInfo>, Error> {
    state
        .store
        .list_commit_signatures(limit.unwrap_or(50))
        .await
}

/// A single commit's signature detail (the per-commit detail sheet).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
async fn get_commit_signature(
    state: State<'_, AppState>,
    hash: String,
) -> Result<CommitSigInfo, Error> {
    state.store.commit_signature(&hash).await
}

// ---------------------------------------------------------------------------
// Timer helpers
// ---------------------------------------------------------------------------

/// Unlock the store with `passphrase` and (re)arm the auto-lock timer.
///
/// Shared by the password UI ([`unlock`]) and the biometric path
/// ([`biometric_unlock`]) so both honor the same "unlock + arm timer"
/// contract — whatever the password flow does, biometric mirrors (plan D5).
async fn unlock_and_arm(
    state: &State<'_, AppState>,
    app: &AppHandle,
    passphrase: &str,
) -> Result<(), Error> {
    state.store.unlock(passphrase).await?;
    reset_lock_timer(state, app);
    // The backend is the single source of truth for lock state; tell the frontend.
    emit_lock_state(app, &state.store).await;
    Ok(())
}

/// Snapshot of the identity lock state, emitted on every lock/unlock transition.
///
/// The frontend's `locked` ref is a pure mirror of this — it must never decide
/// lock state on its own (it used to, after its own `unlock` call, which desynced
/// from the backend on reset and on setup of an encrypted identity).
#[derive(Debug, Clone, Copy, Serialize)]
struct LockState {
    locked: bool,
}

/// Compute the current lock state from the store and emit it as
/// `identity-lock-state`, so the frontend mirrors the backend.
async fn emit_lock_state(app: &AppHandle, store: &Store) {
    let locked = store.is_identity_encrypted().await && !store.is_unlocked();
    let _ = app.emit("identity-lock-state", LockState { locked });
}

/// Reset the auto-lock timer (cancel-and-respawn pattern).
fn reset_lock_timer(state: &State<'_, AppState>, app: &AppHandle) {
    let Ok(mut timer) = state.lock_timer.lock() else {
        return;
    };

    // Cancel existing timer
    if let Some(handle) = timer.take() {
        handle.abort();
    }

    // Bump the generation so any still-in-flight older task self-disarms on wake.
    let generation = state.lock_generation.fetch_add(1, Ordering::SeqCst) + 1;

    // Spawn new timer
    let app_handle = app.clone();
    let store = state.store.clone();
    let pending = state.pending_write.clone();
    let generation_cell = state.lock_generation.clone();

    let handle = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(
            rustpass::store::DEFAULT_LOCK_TIMEOUT_SECS,
        ))
        .await;

        // Stale-task guard: if a newer (re)arm happened while we slept, a fresher
        // unlock is in effect — do not lock/emit. `abort` is not a generation check,
        // so without this a task already past its sleep can fire right after an unlock.
        if generation_cell.load(Ordering::SeqCst) != generation {
            return;
        }

        // Clear any stashed conflict plaintext before wiping the identity —
        // otherwise it would be undecryptable and linger in memory.
        write::clear_pending(&pending);

        // Lock the real store (clears cached identity + passphrase)
        store.lock();

        // Emit the current lock state so the frontend shows the unlock overlay
        // + clears revealed secrets.
        emit_lock_state(&app_handle, &store).await;
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
        .plugin(tauri_plugin_safe_area::init())
        .plugin(tauri_plugin_biometric_keystore::init())
        .plugin(tauri_plugin_file_picker::init())
        .setup(|app| {
            let config_dir = app
                .path()
                .app_config_dir()
                .expect("Cannot determine app config directory");
            app.manage(AppState {
                store: Arc::new(Store::new(config_dir)),
                lock_timer: Mutex::new(None),
                lock_generation: Arc::new(AtomicU64::new(0)),
                pending_identity: Mutex::new(None),
                pending_write: Arc::new(Mutex::new(None)),
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
            pick_identity_file,
            verify_picked_identity,
            complete_setup_from_file,
            clear_pending_identity,
            setup,
            unlock,
            lock,
            set_passphrase,
            change_passphrase,
            is_biometric_available,
            is_biometric_unlock_enabled,
            enable_biometric_unlock,
            biometric_unlock,
            disable_biometric_unlock,
            list_entries,
            pull_repo,
            copy_password,
            show_password,
            get_config,
            reset_config,
            generate_ssh_key,
            get_ssh_public_key,
            export_ssh_private_key,
            get_authenticity_state,
            set_verification_mode,
            get_authenticity_config,
            add_trusted_key,
            remove_trusted_key,
            trust_head_signer,
            trust_commit_signer,
            ignore_commit_issue,
            list_commit_signatures,
            get_commit_signature,
            write::list_create_presets,
            write::lookup_template,
            write::preview_create,
            write::create_secret,
            write::create_from_preset_secret,
            write::resolve_write_conflict,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
