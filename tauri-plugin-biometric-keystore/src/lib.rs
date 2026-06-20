// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Tauri plugin that stores the gpm identity **passphrase** in the Android
//! Keystore (hardware-backed, AES/GCM) and retrieves it through a
//! biometric-gated `BiometricPrompt`.
//!
//! This is a **backend-only** plugin: the frontend never calls it directly.
//! App-layer commands in `src-tauri/src/lib.rs` call
//! [`KeystoreExt::keystore`] to obtain the handle and then `store`/`retrieve`
//! it — the passphrase flows Kotlin → Rust → `Store::unlock` and never reaches
//! the WebView.
//!
//! On desktop (and any non-Android target) the plugin is registered but inert:
//! every operation reports [`KeystoreError::unavailable`], so
//! `is_available`/`has_stored` read `false` and the UI falls back to the
//! passphrase form.

use serde::{Deserialize, Serialize};
use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime};

/// Android package hosting the `KeystorePlugin` Kotlin class.
#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "xyz.yzx9.gpm.biometrickeystore";

// ---------------------------------------------------------------------------
// Error type (unified across mobile/desktop)
// ---------------------------------------------------------------------------

/// Error returned by keystore operations.
///
/// Carries the Kotlin `BIOMETRIC_*` codes through to the app layer. Serializes
/// to `{ code, message }` and **never** contains secret content — messages are
/// derived only from exception class names or system-provided strings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeystoreError {
    /// Machine-readable code, e.g. `BIOMETRIC_UNAVAILABLE`,
    /// `BIOMETRIC_CANCELLED`, `BIOMETRIC_KEY_INVALIDATED`, `BIOMETRIC_FAILED`.
    pub code: String,
    /// Safe (no-secret) human-readable message.
    pub message: String,
}

impl KeystoreError {
    /// "Biometric not available on this platform/device" sentinel.
    #[must_use]
    pub fn unavailable() -> Self {
        Self {
            code: "BIOMETRIC_UNAVAILABLE".to_string(),
            message: "Biometric unlock is not available on this device".to_string(),
        }
    }
}

/// Map a Tauri mobile-plugin invoke error into a [`KeystoreError`],
/// preserving the Kotlin-supplied `BIOMETRIC_*` code when present.
#[cfg(target_os = "android")]
fn map_invoke_err(err: tauri::plugin::mobile::PluginInvokeError) -> KeystoreError {
    use tauri::plugin::mobile::PluginInvokeError;
    match err {
        PluginInvokeError::InvokeRejected(resp) => KeystoreError {
            code: resp.code.unwrap_or_else(|| "BIOMETRIC_FAILED".to_string()),
            message: resp
                .message
                .unwrap_or_else(|| "Biometric operation failed".to_string()),
        },
        other => KeystoreError {
            code: "BIOMETRIC_FAILED".to_string(),
            message: other.to_string(),
        },
    }
}

// ---------------------------------------------------------------------------
// Keystore handle (cfg-gated: real on Android, stub elsewhere)
// ---------------------------------------------------------------------------

/// Handle to the keystore. On Android it wraps the mobile plugin handle; on
/// other targets it is an inert stub whose operations report unavailable.
#[cfg(target_os = "android")]
pub struct Keystore<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// Handle to the keystore — inert stub on non-Android targets.
///
/// `PhantomData<fn() -> R>` keeps the stub `Send + Sync` unconditionally (the
/// `fn() -> R` variance does not inherit R's auto-trait bounds), so it can be
/// managed as app state on every target.
#[cfg(not(target_os = "android"))]
pub struct Keystore<R: Runtime>(std::marker::PhantomData<fn() -> R>);

#[cfg(target_os = "android")]
impl<R: Runtime> Keystore<R> {
    /// Whether biometric-gated storage is usable on this device
    /// (API 30+ with a STRONG biometric enrolled). Fast / non-prompting.
    pub fn is_available(&self) -> Result<bool, KeystoreError> {
        #[derive(Deserialize)]
        struct Resp {
            available: bool,
        }
        self.0
            .run_mobile_plugin::<Resp>("is_available", ())
            .map(|r| r.available)
            .map_err(map_invoke_err)
    }

    /// Whether a stored passphrase exists (non-prompting read of the
    /// ciphertext state in prefs).
    pub fn has_stored(&self) -> Result<bool, KeystoreError> {
        #[derive(Deserialize)]
        struct Resp {
            stored: bool,
        }
        self.0
            .run_mobile_plugin::<Resp>("has_stored", ())
            .map(|r| r.stored)
            .map_err(map_invoke_err)
    }

    /// Delete the stored passphrase and the Keystore key (best-effort).
    pub fn delete(&self) -> Result<(), KeystoreError> {
        self.0
            .run_mobile_plugin::<()>("delete", ())
            .map_err(map_invoke_err)
    }

    /// Seal `passphrase` into the Keystore. **Shows a biometric prompt**
    /// (CryptoObject ENCRYPT) — the key is `setUserAuthenticationRequired`,
    /// so encrypt needs user auth too. Holds the `Invoke` across the prompt,
    /// so it uses the async variant (Finding 7).
    pub async fn store(&self, passphrase: &str) -> Result<(), KeystoreError> {
        #[derive(Serialize)]
        struct Payload<'a> {
            passphrase: &'a str,
        }
        self.0
            .run_mobile_plugin_async::<()>("store", Payload { passphrase })
            .await
            .map_err(map_invoke_err)
    }

    /// Retrieve the sealed passphrase. **Shows a biometric prompt**
    /// (CryptoObject DECRYPT). The passphrase is returned here (Rust side
    /// only) and wrapped in `Zeroizing<String>` by the caller.
    pub async fn retrieve(&self) -> Result<String, KeystoreError> {
        #[derive(Deserialize)]
        struct Resp {
            passphrase: String,
        }
        self.0
            .run_mobile_plugin_async::<Resp>("retrieve", ())
            .await
            .map(|r| r.passphrase)
            .map_err(map_invoke_err)
    }
}

#[cfg(not(target_os = "android"))]
impl<R: Runtime> Keystore<R> {
    /// Inert: biometric is never available on non-Android targets.
    pub fn is_available(&self) -> Result<bool, KeystoreError> {
        Ok(false)
    }

    /// Inert: nothing is ever stored.
    pub fn has_stored(&self) -> Result<bool, KeystoreError> {
        Ok(false)
    }

    /// Inert: nothing to delete.
    pub fn delete(&self) -> Result<(), KeystoreError> {
        Ok(())
    }

    /// Inert: never succeeds — biometric is unavailable.
    pub async fn store(&self, _passphrase: &str) -> Result<(), KeystoreError> {
        Err(KeystoreError::unavailable())
    }

    /// Inert: never succeeds — biometric is unavailable.
    pub async fn retrieve(&self) -> Result<String, KeystoreError> {
        Err(KeystoreError::unavailable())
    }
}

// ---------------------------------------------------------------------------
// Extension trait
// ---------------------------------------------------------------------------

/// Extensions to access the keystore handle from any [`Manager`]
/// (e.g. `AppHandle`).
pub trait KeystoreExt<R: Runtime> {
    /// Obtain the keystore handle. Always present (the plugin is registered on
    /// every target); on non-Android targets the handle is an inert stub.
    fn keystore(&self) -> &Keystore<R>;
}

impl<R: Runtime, T: Manager<R>> KeystoreExt<R> for T {
    fn keystore(&self) -> &Keystore<R> {
        self.state::<Keystore<R>>().inner()
    }
}

// ---------------------------------------------------------------------------
// Plugin initialization
// ---------------------------------------------------------------------------

/// Initializes the keystore plugin.
///
/// On Android, registers the Kotlin `KeystorePlugin` class and manages the
/// handle. On desktop, manages an inert stub so `KeystoreExt::keystore` is
/// always callable (operations report unavailable).
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("biometric-keystore")
        .setup(|app, #[allow(unused_variables)] api| {
            #[cfg(target_os = "android")]
            {
                let handle = api.register_android_plugin(PLUGIN_IDENTIFIER, "KeystorePlugin")?;
                app.manage(Keystore(handle));
            }
            #[cfg(not(target_os = "android"))]
            {
                app.manage(Keystore::<R>(std::marker::PhantomData));
            }
            Ok(())
        })
        .build()
}
