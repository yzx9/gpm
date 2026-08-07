// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Biometric unlock commands — seal/retrieve the identity passphrase behind the
//! Android Keystore's biometric-gated `BiometricPrompt`.

use std::fmt;

use rustpass::Error;
use rustpass::error::ErrorCode;
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_keystore::{BiometricState, KeystoreError, KeystoreExt, PromptText};
use zeroize::Zeroizing;

use crate::AppState;
use crate::identity;
use crate::keystore::{self, PASSPHRASE_ALIAS, PASSPHRASE_POLICY, PASSPHRASE_PREFS};

// ---------------------------------------------------------------------------
// Tauri-IPC types (not in rustpass — these are UI-layer concerns)
// ---------------------------------------------------------------------------

/// App-local error for the biometric commands.
///
/// Serializes to `{ code, message }` — the same shape as `rustpass::Error` —
/// so the frontend can destructure both uniformly. Carries the Kotlin
/// `KEYSTORE_*` codes (via [`From<KeystoreError>`]) and maps
/// `rustpass::Error` (via [`From<Error>`]) so a stale stored passphrase's
/// `WRONG_PASSPHRASE` reaches the frontend. `rustpass::ErrorCode` is not
/// touched; this type lives entirely in the app layer.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct BiometricError {
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

impl From<KeystoreError> for BiometricError {
    fn from(e: KeystoreError) -> Self {
        Self {
            code: e.code,
            message: e.message,
        }
    }
}

impl fmt::Display for BiometricError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Biometric availability as a quad-state [`BiometricState`] (`available` /
/// `no_enrollment` / `weak_enrolled` / `unavailable`), serialized to the
/// frontend as the matching `snake_case` string. `unavailable` on desktop and
/// Android <11. Frontend derives `=== "available"` where it needs a boolean.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn is_biometric_available(
    app: AppHandle,
) -> Result<BiometricState, BiometricError> {
    Ok(app.keystore().is_biometric_available().await?)
}

/// Open the system Security settings (the biometric-enrollment surface) — the
/// recovery target when [`is_biometric_available`] reports `no_enrollment`.
/// Returns whether a handler activity was found. Always `true` on desktop.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn open_security_settings(app: AppHandle) -> Result<bool, BiometricError> {
    Ok(app.keystore().open_security_settings().await)
}

/// Whether a passphrase is sealed in the Keystore — the single source of
/// truth for "biometric is enabled" (no flag file). `false` on desktop.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn is_biometric_unlock_enabled(app: AppHandle) -> Result<bool, BiometricError> {
    Ok(app
        .keystore()
        .has_stored(PASSPHRASE_ALIAS, PASSPHRASE_PREFS)
        .await?)
}

/// Defense-in-depth backstop for the Settings UI gate: refuse biometric
/// enablement when the identity is not passphrase-encrypted. Biometric seals a
/// passphrase, which a plaintext identity has none of, so enabling it there is
/// meaningless — the UI already hides the control, this is the backend guard.
fn require_encrypted_identity(is_encrypted: bool) -> Result<(), Error> {
    if is_encrypted {
        Ok(())
    } else {
        Err(Error::new(
            ErrorCode::IdentityNotEncrypted,
            "Biometric unlock requires a passphrase-encrypted identity",
        ))
    }
}

/// Decode retrieved passphrase bytes to a [`Zeroizing`] UTF-8 string, wiping the
/// bytes on a decode failure. age passphrases are UTF-8, so non-UTF-8 bytes mean
/// a corrupt seal → [`BiometricError`] (`BIOMETRIC_CORRUPT_SLOT`). Wrapping in
/// [`Zeroizing`] before the decode keeps the wipe-everything hygiene on the rare
/// failure path (the bytes are cleared when the [`Zeroizing`] drops, not freed
/// raw). Pure + unit-testable (the `biometric_unlock` command needs an
/// `AppHandle`, so the decode stays here).
fn passphrase_from_bytes(bytes: Vec<u8>) -> Result<Zeroizing<String>, BiometricError> {
    let bytes = Zeroizing::new(bytes);
    match std::str::from_utf8(&bytes) {
        Ok(s) => Ok(Zeroizing::new(s.to_owned())),
        Err(_) => Err(BiometricError {
            code: "BIOMETRIC_CORRUPT_SLOT".to_string(),
            message: "Sealed passphrase slot is corrupt".to_string(),
        }),
    }
}

/// Enable biometric unlock: validate the passphrase, then seal it behind a
/// biometric prompt (encrypt also needs auth for a
/// `setUserAuthenticationRequired` key).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn enable_biometric_unlock(
    state: State<'_, AppState>,
    app: AppHandle,
    passphrase: String,
    prompt_text: Option<PromptText>,
) -> Result<(), BiometricError> {
    log::info!("biometric: enable");
    // Refuse a plaintext identity before sealing anything: biometric seals a
    // passphrase, which a plaintext identity has none of. The Settings UI hides
    // this control for plaintext identities — this is the backend backstop so a
    // UI regression (or a direct IPC call) can't reach the Keystore.
    require_encrypted_identity(state.store.is_identity_encrypted().await)?;
    // Reject a wrong passphrase before sealing it (age or SSH).
    state.store.validate_passphrase(&passphrase).await?;
    // The Kotlin `store` shows a CryptoObject ENCRYPT biometric prompt.
    let resolved = keystore::resolve_prompt(prompt_text.as_ref());
    app.keystore()
        .store(
            passphrase.as_bytes(),
            PASSPHRASE_ALIAS,
            PASSPHRASE_PREFS,
            PASSPHRASE_POLICY,
            Some(&resolved),
        )
        .await?;
    Ok(())
}

/// Unlock via biometrics: retrieve the sealed passphrase and run it through
/// the same `unlock_and_arm` path as the password UI. If the stored passphrase
/// is stale (age path returns `WRONG_PASSPHRASE`), self-heal by deleting it so
/// it stops auto-prompting and the form is revealed for re-enabling.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn biometric_unlock(
    state: State<'_, AppState>,
    app: AppHandle,
    prompt_text: Option<PromptText>,
) -> Result<(), BiometricError> {
    log::info!("biometric: unlock");
    // Flows Kotlin → Rust (never the WebView); wipe as soon as it's used. The
    // bytes go straight to passphrase_from_bytes so they're wrapped in
    // Zeroizing (and wiped on a corrupt-slot decode failure) with no
    // intermediate unwrapped copy.
    let resolved = keystore::resolve_prompt(prompt_text.as_ref());
    let passphrase = passphrase_from_bytes(
        app.keystore()
            .retrieve(
                PASSPHRASE_ALIAS,
                PASSPHRASE_PREFS,
                PASSPHRASE_POLICY,
                Some(&resolved),
            )
            .await?,
    )?;

    if let Err(e) = identity::unlock_and_arm(&state, &app, &passphrase).await {
        if e.code == "WRONG_PASSPHRASE" &&
            // Stale sealed passphrase — clear it so the page reveals the form.
            let Err(cleanup) = app
                .keystore()
                .delete(PASSPHRASE_ALIAS, PASSPHRASE_PREFS)
                .await
        {
            log::warn!("biometric: stale slot cleanup failed: {cleanup:?}");
        }
        return Err(BiometricError::from(e));
    }
    Ok(())
}

/// Disable biometric unlock: best-effort delete the sealed passphrase + key.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn disable_biometric_unlock(app: AppHandle) -> Result<(), BiometricError> {
    log::info!("biometric: disable");
    app.keystore()
        .delete(PASSPHRASE_ALIAS, PASSPHRASE_PREFS)
        .await
        .map_err(|e| {
            let be: BiometricError = e.into();
            log::warn!("biometric: disable failed: {be}");
            be
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn require_encrypted_identity_refuses_plaintext() {
        let err = require_encrypted_identity(false).unwrap_err();
        assert_eq!(err.code, "IDENTITY_NOT_ENCRYPTED");
        assert!(require_encrypted_identity(true).is_ok());
    }

    #[test]
    fn passphrase_from_bytes_accepts_valid_utf8() {
        let p = passphrase_from_bytes(b"hunter2".to_vec()).unwrap();
        assert_eq!(&*p, "hunter2");
    }

    #[test]
    fn passphrase_from_bytes_rejects_non_utf8_as_corrupt_slot() {
        // 0xff/0xfe are never valid UTF-8 start bytes ⇒ corrupt slot.
        let err = passphrase_from_bytes(vec![0xff, 0xfe, 0xc0]).unwrap_err();
        assert_eq!(err.code, "BIOMETRIC_CORRUPT_SLOT");
    }
}
