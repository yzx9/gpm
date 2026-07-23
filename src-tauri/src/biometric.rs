// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Biometric unlock commands — seal/retrieve the identity passphrase behind the
//! Android Keystore's biometric-gated `BiometricPrompt`.

use rustpass::Error;
use rustpass::error::ErrorCode;
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_biometric_keystore::KeystoreExt;
use zeroize::Zeroizing;

use crate::AppState;
use crate::identity::unlock_and_arm;

// ---------------------------------------------------------------------------
// Tauri-IPC types (not in rustpass — these are UI-layer concerns)
// ---------------------------------------------------------------------------

/// App-local error for the biometric commands.
///
/// Serializes to `{ code, message }` — the same shape as `rustpass::Error` —
/// so the frontend can destructure both uniformly. Carries the Kotlin
/// `BIOMETRIC_*` codes (via [`From<KeystoreError>`]) and maps
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

impl From<tauri_plugin_biometric_keystore::KeystoreError> for BiometricError {
    fn from(e: tauri_plugin_biometric_keystore::KeystoreError) -> Self {
        Self {
            code: e.code,
            message: e.message,
        }
    }
}

impl std::fmt::Display for BiometricError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Whether biometric-gated storage is usable on this device (API 30+ with a
/// STRONG biometric enrolled). `false` on desktop and Android <11.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn is_biometric_available(app: AppHandle) -> Result<bool, BiometricError> {
    Ok(app.keystore().is_available().await?)
}

/// Whether a passphrase is sealed in the Keystore — the single source of
/// truth for "biometric is enabled" (no flag file). `false` on desktop.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn is_biometric_unlock_enabled(app: AppHandle) -> Result<bool, BiometricError> {
    Ok(app.keystore().has_stored().await?)
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

/// Enable biometric unlock: validate the passphrase, then seal it behind a
/// biometric prompt (encrypt also needs auth for a
/// `setUserAuthenticationRequired` key).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn enable_biometric_unlock(
    state: State<'_, AppState>,
    app: AppHandle,
    passphrase: String,
    prompt_text: Option<tauri_plugin_biometric_keystore::PromptText>,
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
    app.keystore()
        .store(&passphrase, prompt_text.as_ref())
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
    prompt_text: Option<tauri_plugin_biometric_keystore::PromptText>,
) -> Result<(), BiometricError> {
    log::info!("biometric: unlock");
    // Flows Kotlin → Rust (never the WebView); wipe as soon as it's used.
    let passphrase = Zeroizing::new(app.keystore().retrieve(prompt_text.as_ref()).await?);

    if let Err(e) = unlock_and_arm(&state, &app, &passphrase).await {
        if e.code == "WRONG_PASSPHRASE" {
            // Stale sealed passphrase — clear it so the page reveals the form.
            if let Err(cleanup) = app.keystore().delete().await {
                log::warn!("biometric: stale slot cleanup failed: {cleanup:?}");
            }
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
    app.keystore().delete().await.map_err(|e| {
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
}
