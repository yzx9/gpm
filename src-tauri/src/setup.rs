// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Setup & identity commands — repo clone, identity pick / verify / save, and
//! the auth-state snapshot the router guard reads.

use rustpass::error::ErrorCode;
use rustpass::identity::{IdentityType, classify_identity};
use rustpass::ssh;
use rustpass::{Error, IdentityInfo, KeyType, Recipient};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_file_picker::FilePickerExt;
use zeroize::Zeroizing;

use crate::AppState;
use crate::identity::emit_lock_state;

// ---------------------------------------------------------------------------
// Tauri-IPC types (not in rustpass — these are UI-layer concerns)
// ---------------------------------------------------------------------------

/// Returned by `list_recipients` — public key info for setup step 2.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct RecipientInfo {
    public_key: String,
    comment: Option<String>,
    key_type: String,
}

/// Returned by `get_auth_state` — atomic auth snapshot for router guard.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct AuthState {
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
pub(crate) struct IdentityInfoResult {
    key_type: String,
    encrypted: bool,
}

/// Returned by `pick_identity_file` — identity metadata only. The file contents
/// are held in backend state and never sent to the `WebView`.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct PickedIdentityResult {
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
pub(crate) struct VerifiedIdentityResult {
    recipient: String,
}

/// Which kind of identity [`generate_identity`] should mint for the create flow.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum CreateIdentityKind {
    /// Native x25519 age identity.
    Age,
    /// ed25519 SSH keypair.
    Ssh,
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

/// A file-picked identity awaiting save.
///
/// Does not derive `Debug` — `identity` is secret. Held in `AppState` while the
/// user supplies its passphrase; the frontend only ever sees metadata (via
/// `PickedIdentityResult` / `VerifiedIdentityResult`).
pub(crate) struct PendingIdentity {
    /// The usable identity text `Store::save_identity` receives. For an
    /// age-encrypted upload this is replaced with the decrypted bare key after
    /// verification.
    pub(crate) identity: Zeroizing<String>,
    /// Type + encryption status of the *current* identity (updated to
    /// unencrypted after an age-encrypted identity is decrypted at verify time).
    pub(crate) info: IdentityInfo,
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Get the authentication state as a single atomic snapshot.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn get_auth_state(state: State<'_, AppState>) -> Result<AuthState, Error> {
    let itype = state.store.identity_type().await;
    Ok(AuthState {
        configured: state.store.is_configured(),
        encrypted: state.store.is_identity_encrypted().await,
        unlocked: state.store.is_unlocked(),
        identity_type: identity_type_string(itype),
    })
}

/// Check if the app has been configured (identity + repo exist).
#[tauri::command]
#[allow(clippy::needless_pass_by_value, clippy::unnecessary_wraps)]
pub(crate) fn is_configured(state: State<'_, AppState>) -> Result<bool, Error> {
    Ok(state.store.is_configured())
}

/// Check if the repo has been cloned (step 1 done, identity may be missing).
#[tauri::command]
#[allow(clippy::needless_pass_by_value, clippy::unnecessary_wraps)]
pub(crate) fn is_repo_ready(state: State<'_, AppState>) -> Result<bool, Error> {
    Ok(state.store.is_repo_ready())
}

/// Step 1 of setup: clone the repo and save repo config (no identity).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn clone_repo(
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

/// Mint a fresh identity for the create flow and stage it in backend
/// state — the create-side analogue of [`pick_identity_file`]. Returns **only the
/// public recipient**; the secret identity never reaches the `WebView`. It is
/// saved later by [`complete_setup_from_file`], which consumes the staged copy.
///
/// `Age` mints a native x25519 key; `Ssh` mints an ed25519 keypair, optionally
/// encrypted with `passphrase`. For `Age` the `passphrase` is ignored at mint
/// time (it is applied as at-rest encryption by `complete_setup_from_file`).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn generate_identity(
    state: State<'_, AppState>,
    kind: CreateIdentityKind,
    passphrase: Option<String>,
) -> Result<String, Error> {
    generate_identity_core(&state, kind, passphrase.as_deref())
}

/// Testable core of [`generate_identity`]: mint the identity, stage it in
/// `pending_identity`, return only the recipient. Factored out because the
/// command touches nothing but `pending_identity` (no `AppHandle`, no git), so
/// it can be exercised directly with a minimal [`AppState`] and no Tauri runtime.
pub(crate) fn generate_identity_core(
    state: &AppState,
    kind: CreateIdentityKind,
    passphrase: Option<&str>,
) -> Result<String, Error> {
    let (identity, recipient, info) = match kind {
        CreateIdentityKind::Age => {
            let generated = rustpass::crypto::generate_age_identity();
            (
                generated.identity,
                generated.recipient,
                IdentityInfo {
                    key_type: KeyType::X25519,
                    encrypted: false,
                },
            )
        }
        CreateIdentityKind::Ssh => {
            let pair = ssh::generate_keypair(passphrase)?;
            let encrypted = passphrase.is_some_and(|p| !p.is_empty());
            (
                pair.private_key,
                pair.public_key,
                IdentityInfo {
                    key_type: KeyType::SshEd25519,
                    encrypted,
                },
            )
        }
    };

    // Stage the secret in backend state; the frontend only ever sees the
    // recipient. Overwrites any prior staged identity (matches pick_identity_file).
    {
        let mut guard = state
            .pending_identity
            .lock()
            .expect("pending_identity lock poisoned");
        *guard = Some(PendingIdentity { identity, info });
    }
    Ok(recipient)
}

/// Create a brand-new local gopass store, the create alternative to
/// [`clone_repo`]. Seeds `.age-recipients` with `recipient` and makes the
/// gopass "Initialized Store" commit. When `repo_url` is given it records an
/// `origin` remote (local only).
///
/// Does **not** push — the first push is a separate `push_repo` step performed
/// after [`complete_setup`], so the remote only receives the store once its
/// identity is durable (closes the orphan-recipient hole). Auth fields are
/// ignored when no `repo_url` is given.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn create_store(
    state: State<'_, AppState>,
    repo_url: Option<String>,
    pat: Option<String>,
    ssh_key: Option<String>,
    ssh_passphrase: Option<String>,
    recipient: String,
) -> Result<(), Error> {
    state
        .store
        .create_store(
            repo_url.as_deref(),
            pat.as_deref(),
            ssh_key.as_deref(),
            ssh_passphrase.as_deref(),
            &recipient,
        )
        .await
}

/// Read recipients from the cloned repository for setup step 2.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn list_recipients(
    state: State<'_, AppState>,
) -> Result<Vec<RecipientInfo>, Error> {
    let recipients = state.store.list_recipients().await?;
    Ok(recipients.into_iter().map(RecipientInfo::from).collect())
}

/// Validate an identity and return its type and encryption status.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn validate_identity(identity: String) -> Result<IdentityInfoResult, Error> {
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
pub(crate) async fn complete_setup(
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
pub(crate) async fn pick_identity_file(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<PickedIdentityResult, Error> {
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
/// decrypted to the bare key). On failure the pending identity is **dropped**
/// (the file is abandoned) and `WRONG_PASSPHRASE` is returned.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn verify_picked_identity(
    state: State<'_, AppState>,
    passphrase: String,
) -> Result<VerifiedIdentityResult, Error> {
    verify_picked(&state, passphrase).await
}

/// Core of [`verify_picked_identity`], taking `&AppState` directly so the
/// pending-identity state machine (success re-stores, any failure abandons the
/// file) is testable without a Tauri `State` extractor.
pub(crate) async fn verify_picked(
    state: &AppState,
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
pub(crate) async fn complete_setup_from_file(
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
pub(crate) fn clear_pending_identity(state: State<'_, AppState>) -> Result<(), Error> {
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
pub(crate) async fn setup(
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Map an [`IdentityType`](rustpass::identity::IdentityType) to a stable string
/// for IPC, matching the `key_type` values returned by [`validate_identity`].
fn identity_type_string(itype: IdentityType) -> String {
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

#[cfg(test)]
mod tests {
    use super::{generate_identity_core, CreateIdentityKind};
    use crate::AppState;
    use rustpass::{KeyType, Store};
    use std::sync::atomic::AtomicU64;
    use std::sync::{Arc, Mutex};

    /// Minimal [`AppState`]. `generate_identity_core` only touches
    /// `pending_identity`, so the store / timer / pending-write are inert
    /// placeholders — no git repo or Tauri runtime needed.
    fn pending_state() -> (AppState, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = AppState {
            store: Arc::new(Store::new(dir.path().to_path_buf(), None)),
            lock_timer: Mutex::new(None),
            lock_generation: Arc::new(AtomicU64::new(0)),
            pending_identity: Mutex::new(None),
            pending_write: Arc::new(Mutex::new(None)),
        };
        (state, dir)
    }

    #[test]
    fn age_mints_and_stages_the_matching_secret() {
        let (state, _dir) = pending_state();
        let recipient =
            generate_identity_core(&state, CreateIdentityKind::Age, None).expect("age mint");
        assert!(recipient.starts_with("age1"), "recipient: {recipient}");

        let pending = state
            .pending_identity
            .lock()
            .expect("lock")
            .take()
            .expect("identity staged");
        assert_eq!(pending.info.key_type, KeyType::X25519);
        assert!(!pending.info.encrypted);
        // The staged secret is the matching private key — never the recipient.
        assert!(
            pending.identity.as_str().starts_with("AGE-SECRET-KEY-1"),
            "staged identity: {}",
            pending.identity.as_str()
        );
    }

    #[test]
    fn ssh_encryption_flag_tracks_the_passphrase() {
        let (state, _dir) = pending_state();

        let recipient = generate_identity_core(
            &state,
            CreateIdentityKind::Ssh,
            Some("create-pass"),
        )
        .expect("ssh mint");
        assert!(
            recipient.starts_with("ssh-ed25519 "),
            "recipient: {recipient}"
        );
        let pending = state
            .pending_identity
            .lock()
            .expect("lock")
            .take()
            .expect("identity staged");
        assert_eq!(pending.info.key_type, KeyType::SshEd25519);
        assert!(
            pending.info.encrypted,
            "a passphrase-protected SSH key must be flagged encrypted"
        );
        assert!(
            pending.identity.as_str().contains("BEGIN OPENSSH PRIVATE KEY"),
            "staged identity should be a PEM"
        );

        // Without a passphrase → not encrypted.
        generate_identity_core(&state, CreateIdentityKind::Ssh, None).unwrap();
        let pending = state
            .pending_identity
            .lock()
            .expect("lock")
            .take()
            .expect("identity staged");
        assert!(!pending.info.encrypted);
    }

    #[test]
    fn a_second_mint_overwrites_the_prior_staged_identity() {
        let (state, _dir) = pending_state();
        generate_identity_core(&state, CreateIdentityKind::Age, None).unwrap();
        generate_identity_core(&state, CreateIdentityKind::Ssh, None).unwrap();
        let pending = state
            .pending_identity
            .lock()
            .expect("lock")
            .take()
            .expect("identity staged");
        // The most recent mint wins (matches pick_identity_file's overwrite).
        assert_eq!(pending.info.key_type, KeyType::SshEd25519);
    }
}
