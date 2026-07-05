// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! App-launch biometric gate (RFC 0028) — an opt-in lock that re-seals the
//! at-rest master key behind a biometric-gated Keystore key, so the whole store
//! is unreadable until the user authenticates on launch/resume.
//!
//! This is a **third**, UI/session-layer lock, deliberately independent of the
//! identity cache lock (`identity::`) and of the auth-free at-rest master key:
//! - Enabling migrates the master key from the auth-free store to the
//!   biometric-gated store (and back on disable). The key's location IS the
//!   toggle state — probed non-promptingly at startup, before `repo.json`.
//! - `app_unlock` retrieves the master key via a biometric prompt and injects
//!   it into the `Store`; `app_lock` wipes it (and the identity cache) so a
//!   locked app cannot read the store even from memory.
//! - While the gate is active the frontend suppresses the identity overlay, so
//!   the two never race to show competing prompts.
//!
//! The identity-auto-unlock opt-in (one app-unlock also unlocks the identity)
//! layers on top: the identity passphrase is sealed under the master key, so
//! `app_unlock` decrypts it with no second prompt when the opt-in is on.

use std::sync::atomic::Ordering;

use rustpass::Error;
use rustpass::error::ErrorCode;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime, State};
use tauri_plugin_secure_keystore::SecureKeystoreExt;
use zeroize::Zeroizing;

use crate::AppState;
use crate::{decode_master_key, identity};

// ---------------------------------------------------------------------------
// Tauri-IPC types
// ---------------------------------------------------------------------------

/// App-lock error — serializes to `{ code, message }` (same shape as
/// `rustpass::Error` / `BiometricError`) so the frontend destructures all
/// uniformly. Carries the plugin's `BIOMETRIC_*` / `SECURE_KEYSTORE_*` codes and
/// maps `rustpass::Error` for the config writes.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct AppLockError {
    code: String,
    message: String,
}

impl AppLockError {
    /// Build a generic `APP_LOCK_FAILED` error with a safe (no-secret) message.
    #[must_use]
    fn failed(message: &str) -> Self {
        Self {
            code: "APP_LOCK_FAILED".to_string(),
            message: message.to_string(),
        }
    }
}

impl From<Error> for AppLockError {
    fn from(e: Error) -> Self {
        Self {
            code: e.code,
            message: e.message,
        }
    }
}

impl From<tauri_plugin_secure_keystore::SecureKeystoreError> for AppLockError {
    fn from(e: tauri_plugin_secure_keystore::SecureKeystoreError) -> Self {
        Self {
            code: e.code,
            message: e.message,
        }
    }
}

/// Snapshot of the app-lock state, emitted as `app-lock-state` on every
/// transition and returned by `get_app_lock_state`.
#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct AppLockState {
    /// Whether the gate is enabled (master key lives in the biometric-gated
    /// store).
    enabled: bool,
    /// Whether the app is currently locked (master key not in memory).
    locked: bool,
}

/// Emit the current app-lock state so the frontend mirrors it.
fn emit_app_lock_state<R: Runtime>(app: &AppHandle<R>, enabled: bool, locked: bool) {
    let _ = app.emit("app-lock-state", AppLockState { enabled, locked });
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Whether the app-launch biometric gate is usable on this device (API 30+ with
/// a STRONG biometric). `false` on desktop / Android <11. Gates the Settings
/// toggle.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn is_app_lock_available(app: AppHandle) -> Result<bool, AppLockError> {
    Ok(app.secure_keystore().is_biometric_available().await?)
}

/// Current app-lock state, for the frontend's initial render.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn get_app_lock_state(state: State<'_, AppState>) -> AppLockState {
    AppLockState {
        enabled: state.app_lock_enabled.load(Ordering::SeqCst),
        locked: state.app_locked.load(Ordering::SeqCst),
    }
}

/// Enable the app-launch biometric gate: migrate the master key from the
/// auth-free Keystore store to the biometric-gated one. The biometric prompt
/// (ENCRYPT) runs first; only on its success is the auth-free copy deleted, so a
/// cancel never orphans the store. The in-memory master key is unchanged (same
/// bytes), so the session keeps working.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn enable_biometric_app_lock(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), AppLockError> {
    let ks = app.secure_keystore();
    if !ks.is_biometric_available().await? {
        return Err(AppLockError::from(
            tauri_plugin_secure_keystore::SecureKeystoreError::unavailable(),
        ));
    }
    // Already enabled (key already biometric-gated) — nothing to migrate.
    if ks.has_stored_biometric().await? {
        state.app_lock_enabled.store(true, Ordering::SeqCst);
        return Ok(());
    }

    // Read the current auth-free master key (non-prompting). This is the value
    // we re-seal; wipe it as soon as it's copied into the biometric store.
    let b64 = Zeroizing::new(
        ks.retrieve()
            .await?
            .ok_or_else(|| AppLockError::failed("No at-rest master key to migrate"))?,
    );

    // Seal behind biometric FIRST (prompt). If the user cancels, the auth-free
    // key is untouched — no bricking.
    ks.store_biometric(&b64).await?;
    // Only now drop the auth-free copy and persist the flag.
    ks.delete().await?;
    state.store.set_biometric_app_lock(true).await?;
    state.app_lock_enabled.store(true, Ordering::SeqCst);
    Ok(())
}

/// Disable the app-launch biometric gate: migrate the master key back to the
/// auth-free store (one last biometric DECRYPT prompt), then drop the
/// biometric-gated copy. If the biometric key is dead (all biometrics removed),
/// the master key is unrecoverable and this fails — re-setup is the only path.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn disable_biometric_app_lock(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), AppLockError> {
    let ks = app.secure_keystore();
    // Retrieve the master key from the biometric store (prompt DECRYPT).
    let b64 = Zeroizing::new(
        ks.retrieve_biometric()
            .await?
            .ok_or_else(|| AppLockError::failed("No biometric master key to migrate back"))?,
    );
    // Re-seal into the auth-free store (non-prompting), then drop the biometric
    // copy. The Store's in-memory master key may have been wiped by a prior
    // `app_lock` (disable can be invoked while locked, before a frontend guard
    // exists) — re-inject it BEFORE clearing the flag, since the flag write
    // seals `repo.json` via AtRest::unseal and would fail with
    // `AtRestKeyUnavailable` if the key were still absent.
    ks.store(&b64).await?;
    ks.delete_biometric().await?;
    if let Some(key) = decode_master_key(&b64) {
        state.store.set_master_key(Some(key));
    }
    state.store.set_biometric_app_lock(false).await?;
    // The identity-auto-unlock opt-in is meaningless without the gate (app_unlock
    // is never called when app_lock_enabled is false). Clear its flag + sealed
    // passphrase slot so re-enabling the gate later starts clean — otherwise the
    // persisted flag would silently re-activate auto-unlock with the old sealed
    // passphrase, and the Settings UI (which hides the opt-in while the gate is
    // off) would offer no way to clear it.
    let _ = state.store.clear_app_identity_pass().await;
    let _ = state.store.set_unlock_identity_with_app(false).await;

    state.app_lock_enabled.store(false, Ordering::SeqCst);
    state.app_locked.store(false, Ordering::SeqCst);
    emit_app_lock_state(&app, false, false);
    Ok(())
}

/// Unlock the app: retrieve the master key via a biometric prompt and inject it
/// into the `Store`. The identity cache is left wiped (re-established lazily by
/// per-operation auth, or by the identity-auto-unlock opt-in); a soft
/// identity-lock event tells the frontend the next identity-needing op will
/// re-authenticate WITHOUT raising the identity overlay over the just-unlocked
/// app.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn app_unlock(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), AppLockError> {
    // Idempotent: if already unlocked (or app-lock is off), skip the biometric
    // prompt entirely. Guards against a double-call re-prompting.
    if !state.app_locked.load(Ordering::SeqCst) {
        return Ok(());
    }
    let ks = app.secure_keystore();
    let b64 = Zeroizing::new(
        ks.retrieve_biometric()
            .await?
            .ok_or_else(|| AppLockError::failed("No biometric master key stored"))?,
    );
    let key = decode_master_key(&b64)
        .ok_or_else(|| AppLockError::failed("Stored master key is malformed"))?;
    state.store.set_master_key(Some(key));
    state.app_locked.store(false, Ordering::SeqCst);
    let enabled = state.app_lock_enabled.load(Ordering::SeqCst);
    emit_app_lock_state(&app, enabled, false);
    // Identity-auto-unlock opt-in: if on and the identity is encrypted, unlock
    // it now with the passphrase sealed under the (just-injected) master key —
    // no second prompt. unlock_and_arm emits identity-lock-state{locked:false}.
    // Otherwise, only for a passphrase-encrypted identity, a SOFT identity event
    // tells the frontend to use per-op auth (no overlay over the just-unlocked
    // app). A plaintext identity is always readable straight from disk, so it
    // must NOT receive a soft event — that would force runWithAuth to raise an
    // unusable UnlockModal (no passphrase to enter) on every copy/show.
    if !try_identity_auto_unlock(&state, &app).await && state.store.is_identity_encrypted().await {
        identity::emit_lock_state(&app, &state.store, true).await;
    }
    Ok(())
}

/// Attempt the identity-auto-unlock opt-in after the master key is in memory.
/// Returns `true` if the identity session is now unlocked. Cheaply skips when
/// the opt-in is off or the identity isn't passphrase-encrypted; on a missing
/// slot returns `false` (per-op auth). On a stale sealed passphrase
/// (`WRONG_PASSPHRASE` — the user changed it), self-heals by clearing the slot +
/// the opt-in so it stops auto-attempting, and returns `false`.
async fn try_identity_auto_unlock<R: Runtime>(
    state: &State<'_, AppState>,
    app: &AppHandle<R>,
) -> bool {
    let Ok(rc) = state.store.config().await else {
        return false;
    };
    if !rc.unlock_identity_with_app {
        return false;
    }
    if !state.store.is_identity_encrypted().await {
        return false;
    }
    let Ok(pass_bytes) = state.store.load_app_identity_pass().await else {
        return false; // slot absent, or the master key is somehow unavailable
    };
    // age passphrases are UTF-8; an invalid sequence means a corrupt slot.
    let pass = match std::str::from_utf8(pass_bytes.as_slice()) {
        Ok(s) => Zeroizing::new(s.to_owned()),
        Err(_) => return false,
    };
    match identity::unlock_and_arm(state, app, pass.as_str()).await {
        Ok(()) => true,
        Err(e) => {
            if e.code == "WRONG_PASSPHRASE" {
                let _ = state.store.clear_app_identity_pass().await;
                let _ = state.store.set_unlock_identity_with_app(false).await;
            }
            false
        }
    }
}

/// Lock the app: wipe the master key (the store becomes unreadable) and the
/// identity cache. Emitted as a hard app-lock transition so the frontend raises
/// the app-lock overlay (which suppresses the identity overlay).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn app_lock(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), AppLockError> {
    // Wipe the master key (the store becomes unreadable) and the identity cache.
    // In-flight writes are intentionally allowed to finish: they hold only the
    // already-captured identity bytes (git ops never touch the at-rest master
    // key), and any at-rest read/write racing this wipe surfaces a clean
    // `AtRestKeyUnavailable` (never a silent plaintext downgrade — the
    // `ever_keyed` latch guards `seal`). Do not add a mutex here to "fix" that —
    // it would deadlock the write path.
    state.store.set_master_key(None);
    state.store.lock();
    state.app_locked.store(true, Ordering::SeqCst);
    let enabled = state.app_lock_enabled.load(Ordering::SeqCst);
    emit_app_lock_state(&app, enabled, true);
    Ok(())
}

/// Enable the identity-auto-unlock opt-in: validate `passphrase`, then seal it
/// under the at-rest master key so a later `app_unlock` can unlock the identity
/// with no second prompt. Requires the gate to be enabled and the identity to be
/// passphrase-encrypted (the slot seals a passphrase, which a plaintext identity
/// has none of). The master key must be in memory (i.e. the app is unlocked) for
/// the seal to succeed.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn enable_identity_auto_unlock(
    state: State<'_, AppState>,
    passphrase: String,
) -> Result<(), AppLockError> {
    if !state.app_lock_enabled.load(Ordering::SeqCst) {
        return Err(AppLockError::failed(
            "Enable the app lock before identity auto-unlock",
        ));
    }
    if !state.store.is_identity_encrypted().await {
        return Err(AppLockError::from(Error::new(
            ErrorCode::IdentityNotEncrypted,
            "Identity auto-unlock requires a passphrase-encrypted identity",
        )));
    }
    let passphrase = Zeroizing::new(passphrase);
    // Validate before sealing (rejects a wrong passphrase before it is stored).
    state.store.validate_passphrase(passphrase.as_str()).await?;
    state
        .store
        .save_app_identity_pass(passphrase.as_str())
        .await?;
    state.store.set_unlock_identity_with_app(true).await?;
    Ok(())
}

/// Disable the identity-auto-unlock opt-in: clear the sealed passphrase slot and
/// the flag. Never fails on a missing slot (best-effort clear).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn disable_identity_auto_unlock(
    state: State<'_, AppState>,
) -> Result<(), AppLockError> {
    state.store.clear_app_identity_pass().await?;
    state.store.set_unlock_identity_with_app(false).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_lock_state_serializes() {
        let s = AppLockState {
            enabled: true,
            locked: false,
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"enabled\":true"));
        assert!(json.contains("\"locked\":false"));
    }

    #[test]
    fn app_lock_error_from_rustpass_preserves_code() {
        let err = AppLockError::from(Error::new(ErrorCode::StoreError, "boom"));
        assert_eq!(err.code, "STORE_ERROR");
        assert_eq!(err.message, "boom");
    }

    #[test]
    fn app_lock_error_from_secure_keystore_preserves_code() {
        let err =
            AppLockError::from(tauri_plugin_secure_keystore::SecureKeystoreError::unavailable());
        assert_eq!(err.code, "SECURE_KEYSTORE_UNAVAILABLE");
    }

    #[test]
    fn failed_error_uses_app_lock_failed_code() {
        let err = AppLockError::failed("no key");
        assert_eq!(err.code, "APP_LOCK_FAILED");
        assert_eq!(err.message, "no key");
    }
}
