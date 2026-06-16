// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Identity & access — the age/SSH identity, its locked runtime session, and
//! the auto-lock timer.
//!
//! Owns unlock/lock, passphrase management, SSH key material, and the shared
//! "activity defers auto-lock" plumbing (`reset_lock_timer` / `emit_lock_state`)
//! that `read`, `write`, `config`, and `setup` reuse.

use std::sync::atomic::Ordering;
use std::time::Duration;

use rustpass::ssh;
use rustpass::{Error, ErrorCode, Store};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_biometric_keystore::KeystoreExt;

use crate::{AppState, write};

// ---------------------------------------------------------------------------
// Tauri-IPC types (not in rustpass — these are UI-layer concerns)
// ---------------------------------------------------------------------------

/// Returned by `generate_ssh_key` — contains both keys for setup form.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct SshKeyPairResult {
    public_key: String,
    private_key: String,
}

/// Returned by `get_ssh_public_key` — public key only, safe to display.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct SshPublicKeyResult {
    public_key: String,
}

/// Returned by `export_ssh_private_key` — secret, strict Vue lifecycle required.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct SshPrivateKeyResult {
    private_key: String,
}

// ---------------------------------------------------------------------------
// Lock-state plumbing
// ---------------------------------------------------------------------------

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
pub(crate) async fn emit_lock_state(app: &AppHandle, store: &Store) {
    let locked = store.is_identity_encrypted().await && !store.is_unlocked();
    let _ = app.emit("identity-lock-state", LockState { locked });
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Unlock a passphrase-encrypted identity (async — scrypt is slow).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn unlock(
    state: State<'_, AppState>,
    app: AppHandle,
    passphrase: String,
) -> Result<(), Error> {
    unlock_and_arm(&state, &app, &passphrase).await
}

/// Lock the store: clear cached identity and cancel auto-lock timer.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn lock(state: State<'_, AppState>, app: AppHandle) -> Result<(), Error> {
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
pub(crate) async fn set_passphrase(
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
pub(crate) async fn change_passphrase(
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

/// Generate a new ed25519 SSH keypair for setup.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn generate_ssh_key(passphrase: Option<String>) -> Result<SshKeyPairResult, Error> {
    let pair = ssh::generate_keypair(passphrase.as_deref())?;
    Ok(SshKeyPairResult {
        public_key: pair.public_key,
        private_key: pair.private_key.to_string(),
    })
}

/// Get the public key derived from the stored SSH private key.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn get_ssh_public_key(
    state: State<'_, AppState>,
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
pub(crate) async fn export_ssh_private_key(
    state: State<'_, AppState>,
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
// Auto-lock timer (shared: activity in read/write defers it)
// ---------------------------------------------------------------------------

/// Unlock the store with `passphrase` and (re)arm the auto-lock timer.
///
/// Shared by the password UI ([`unlock`]) and the biometric path
/// (`biometric::biometric_unlock`) so both honor the same "unlock + arm timer"
/// contract — whatever the password flow does, biometric mirrors (plan D5).
pub(crate) async fn unlock_and_arm(
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

/// Reset the auto-lock timer (cancel-and-respawn pattern).
pub(crate) fn reset_lock_timer(state: &State<'_, AppState>, app: &AppHandle) {
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
