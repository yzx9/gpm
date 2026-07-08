// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Tauri plugin that stores the gpm at-rest **master key** in the Android
//! Keystore — sealed with a hardware-backed, **auth-free** AES/GCM key — and
//! hands it back to Rust so `rustpass` can AEAD-encrypt local private files
//! (`repo.json`, `identity`).
//!
//! This is the auth-free sibling of `tauri-plugin-biometric-keystore` (which
//! seals the identity *passphrase* behind a biometric-gated key). Same Keystore
//! AES/GCM mechanism, different key policy:
//! - `biometric-keystore`: `setUserAuthenticationRequired(true)`, per-use
//!   biometric prompt, invalidated on fingerprint-enrollment change.
//! - `secure-keystore` (here): `setUserAuthenticationRequired(false)`, no
//!   prompt, **survives** fingerprint changes — so the at-rest store never
//!   bricks on a fingerprint change.
//!
//! The master key is a random 32-byte secret; the plugin seals it (iv +
//! ciphertext in SharedPreferences) and returns the **plaintext** bytes
//! (Base64 over IPC) to Rust, exactly as `biometric-keystore` returns the
//! passphrase. The non-extractable Keystore key never leaves the secure
//! element; the master key it wraps is no more sensitive than the PAT `rustpass`
//! already holds in memory.
//!
//! This is a **backend-only** plugin: the app layer calls
//! [`SecureKeystoreExt::secure_keystore`] to obtain the handle and retrieve /
//! store the master key — it never reaches the WebView.
//!
//! On non-Android targets the plugin is registered but inert:
//! `is_available` reads `false`, `retrieve` returns `None`, so the app falls
//! back to plaintext at-rest storage (documented asymmetry).

use serde::{Deserialize, Serialize};
#[cfg(target_os = "android")]
use tauri::plugin::mobile::PluginInvokeError;
use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime};

/// Android package hosting the `SecureKeystorePlugin` Kotlin class.
#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "xyz.yzx9.gpm.securekeystore";

// ---------------------------------------------------------------------------
// Error type (unified across mobile/desktop)
// ---------------------------------------------------------------------------

/// Error returned by secure-keystore operations.
///
/// Carries the Kotlin `SECURE_KEYSTORE_*` codes through to the app layer.
/// Serializes to `{ code, message }` and **never** contains secret content —
/// messages are derived only from exception class names or system strings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecureKeystoreError {
    /// Machine-readable code, e.g. `SECURE_KEYSTORE_UNAVAILABLE`,
    /// `SECURE_KEYSTORE_NOT_SET`, `SECURE_KEYSTORE_FAILED`.
    pub code: String,
    /// Safe (no-secret) human-readable message.
    pub message: String,
}

impl SecureKeystoreError {
    /// "Secure keystore not available on this platform/device" sentinel.
    #[must_use]
    pub fn unavailable() -> Self {
        Self {
            code: "SECURE_KEYSTORE_UNAVAILABLE".to_string(),
            message: "Secure keystore is not available on this device".to_string(),
        }
    }
}

/// Map a Tauri mobile-plugin invoke error into a [`SecureKeystoreError`],
/// preserving the Kotlin-supplied code when present.
#[cfg(target_os = "android")]
fn map_invoke_err(err: PluginInvokeError) -> SecureKeystoreError {
    match err {
        PluginInvokeError::InvokeRejected(resp) => SecureKeystoreError {
            code: resp
                .code
                .unwrap_or_else(|| "SECURE_KEYSTORE_FAILED".to_string()),
            message: resp
                .message
                .unwrap_or_else(|| "Secure keystore operation failed".to_string()),
        },
        other => SecureKeystoreError {
            code: "SECURE_KEYSTORE_FAILED".to_string(),
            message: other.to_string(),
        },
    }
}

// ---------------------------------------------------------------------------
// Prompt text
// ---------------------------------------------------------------------------

/// Localized `BiometricPrompt` text supplied by the frontend, so the native
/// layer never localizes. Deserialized from the `{ title, subtitle, negative }`
/// shape the WebView sends and forwarded to Kotlin, which falls back to a
/// generic safety string when a field is absent. Defined here (not the app
/// crate) so the app command's IPC param type IS the plugin's type.
#[derive(Debug, Clone, Deserialize)]
pub struct PromptText {
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub negative: Option<String>,
}

// ---------------------------------------------------------------------------
// Handle (cfg-gated: real on Android, stub elsewhere)
// ---------------------------------------------------------------------------

/// Handle to the secure keystore. On Android it wraps the mobile plugin handle;
/// on other targets it is an inert stub whose operations report unavailable.
#[cfg(target_os = "android")]
pub struct SecureKeystore<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// Handle to the secure keystore — inert stub on non-Android targets.
///
/// `PhantomData<fn() -> R>` keeps the stub `Send + Sync` unconditionally (the
/// `fn() -> R` variance does not inherit R's auto-trait bounds), so it can be
/// managed as app state on every target.
#[cfg(not(target_os = "android"))]
pub struct SecureKeystore<R: Runtime>(std::marker::PhantomData<fn() -> R>);

#[cfg(target_os = "android")]
impl<R: Runtime> SecureKeystore<R> {
    /// Whether the secure keystore is usable on this device. Fast / non-prompting.
    pub async fn is_available(&self) -> Result<bool, SecureKeystoreError> {
        #[derive(Deserialize)]
        struct Resp {
            available: bool,
        }
        self.0
            .run_mobile_plugin_async::<Resp>("isAvailable", ())
            .await
            .map(|r| r.available)
            .map_err(map_invoke_err)
    }

    /// Retrieve the sealed master key (Base64), or `None` if nothing is sealed.
    /// Non-prompting (the key is auth-free).
    pub async fn retrieve(&self) -> Result<Option<String>, SecureKeystoreError> {
        #[derive(Deserialize)]
        struct Resp {
            stored: bool,
            key: Option<String>,
        }
        let r = self
            .0
            .run_mobile_plugin_async::<Resp>("retrieve", ())
            .await
            .map_err(map_invoke_err)?;
        Ok(if r.stored { r.key } else { None })
    }

    /// Seal the supplied master key (Base64) into the Keystore.
    pub async fn store(&self, key_b64: &str) -> Result<(), SecureKeystoreError> {
        #[derive(Serialize)]
        struct Payload<'a> {
            key: &'a str,
        }
        self.0
            .run_mobile_plugin_async::<()>("store", Payload { key: key_b64 })
            .await
            .map_err(map_invoke_err)
    }

    /// Delete the Keystore key and the stored ciphertext (best-effort).
    pub async fn delete(&self) -> Result<(), SecureKeystoreError> {
        self.0
            .run_mobile_plugin_async::<()>("delete", ())
            .await
            .map_err(map_invoke_err)
    }

    /// Whether STRONG biometric auth is usable on this device (API 30+ with a
    /// fingerprint/face enrolled). Fast / non-prompting. Gates the app-lock:
    /// the toggle is only offered when this is `true`.
    pub async fn is_biometric_available(&self) -> Result<bool, SecureKeystoreError> {
        #[derive(Deserialize)]
        struct Resp {
            available: bool,
        }
        self.0
            .run_mobile_plugin_async::<Resp>("isBiometricAvailable", ())
            .await
            .map(|r| r.available)
            .map_err(map_invoke_err)
    }

    /// Whether a biometric-gated master key exists AND its key still inits
    /// cleanly (non-prompting liveness probe). This is the authoritative
    /// "app-lock is enabled" signal at startup — readable before `repo.json`,
    /// since the master key's location (auth-free vs biometric-gated) is itself
    /// the toggle state. `false` if biometrics were removed and the key is dead.
    pub async fn has_stored_biometric(&self) -> Result<bool, SecureKeystoreError> {
        #[derive(Deserialize)]
        struct Resp {
            stored: bool,
        }
        self.0
            .run_mobile_plugin_async::<Resp>("hasStoredBiometric", ())
            .await
            .map(|r| r.stored)
            .map_err(map_invoke_err)
    }

    /// Seal the supplied master key (Base64) behind a biometric-gated key.
    /// **Shows a BiometricPrompt** (CryptoObject ENCRYPT). Used when enabling the
    /// app-lock to migrate the master key from the auth-free store. `prompt`
    /// supplies the localized prompt text.
    pub async fn store_biometric(
        &self,
        key_b64: &str,
        prompt: Option<&PromptText>,
    ) -> Result<(), SecureKeystoreError> {
        #[derive(Serialize)]
        struct Payload<'a> {
            key: &'a str,
            title: Option<&'a str>,
            subtitle: Option<&'a str>,
            negative: Option<&'a str>,
        }
        self.0
            .run_mobile_plugin_async::<()>(
                "storeBiometric",
                Payload {
                    key: key_b64,
                    title: prompt.and_then(|p| p.title.as_deref()),
                    subtitle: prompt.and_then(|p| p.subtitle.as_deref()),
                    negative: prompt.and_then(|p| p.negative.as_deref()),
                },
            )
            .await
            .map_err(map_invoke_err)
    }

    /// Retrieve the biometric-gated master key (Base64), or `None` if nothing is
    /// sealed. **Shows a BiometricPrompt** (CryptoObject DECRYPT). The key is
    /// returned here (Rust side only). Rejects with `BIOMETRIC_KEY_INVALIDATED`
    /// if the key died (all biometrics removed). `prompt` supplies the localized
    /// prompt text.
    pub async fn retrieve_biometric(
        &self,
        prompt: Option<&PromptText>,
    ) -> Result<Option<String>, SecureKeystoreError> {
        #[derive(Deserialize)]
        struct Resp {
            stored: bool,
            key: Option<String>,
        }
        #[derive(Serialize)]
        struct Payload<'a> {
            title: Option<&'a str>,
            subtitle: Option<&'a str>,
            negative: Option<&'a str>,
        }
        let r = self
            .0
            .run_mobile_plugin_async::<Resp>(
                "retrieveBiometric",
                Payload {
                    title: prompt.and_then(|p| p.title.as_deref()),
                    subtitle: prompt.and_then(|p| p.subtitle.as_deref()),
                    negative: prompt.and_then(|p| p.negative.as_deref()),
                },
            )
            .await
            .map_err(map_invoke_err)?;
        Ok(if r.stored { r.key } else { None })
    }

    /// Delete the biometric-gated Keystore key and ciphertext (best-effort).
    /// Used when disabling the app-lock (after the master key is migrated back
    /// to the auth-free store).
    pub async fn delete_biometric(&self) -> Result<(), SecureKeystoreError> {
        self.0
            .run_mobile_plugin_async::<()>("deleteBiometric", ())
            .await
            .map_err(map_invoke_err)
    }
}

#[cfg(not(target_os = "android"))]
impl<R: Runtime> SecureKeystore<R> {
    /// Inert: the secure keystore is never available on non-Android targets.
    pub async fn is_available(&self) -> Result<bool, SecureKeystoreError> {
        Ok(false)
    }

    /// Inert: nothing is ever stored.
    pub async fn retrieve(&self) -> Result<Option<String>, SecureKeystoreError> {
        Ok(None)
    }

    /// Inert: never succeeds — the secure keystore is unavailable.
    pub async fn store(&self, _key_b64: &str) -> Result<(), SecureKeystoreError> {
        Err(SecureKeystoreError::unavailable())
    }

    /// Inert: nothing to delete.
    pub async fn delete(&self) -> Result<(), SecureKeystoreError> {
        Ok(())
    }

    /// Inert: biometric is never available on non-Android targets.
    pub async fn is_biometric_available(&self) -> Result<bool, SecureKeystoreError> {
        Ok(false)
    }

    /// Inert: no biometric-gated key is ever stored.
    pub async fn has_stored_biometric(&self) -> Result<bool, SecureKeystoreError> {
        Ok(false)
    }

    /// Inert: never succeeds — biometric is unavailable.
    pub async fn store_biometric(
        &self,
        _key_b64: &str,
        _prompt: Option<&PromptText>,
    ) -> Result<(), SecureKeystoreError> {
        Err(SecureKeystoreError::unavailable())
    }

    /// Inert: nothing is ever stored.
    pub async fn retrieve_biometric(
        &self,
        _prompt: Option<&PromptText>,
    ) -> Result<Option<String>, SecureKeystoreError> {
        Ok(None)
    }

    /// Inert: nothing to delete.
    pub async fn delete_biometric(&self) -> Result<(), SecureKeystoreError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Extension trait
// ---------------------------------------------------------------------------

/// Extensions to access the secure-keystore handle from any [`Manager`]
/// (e.g. `AppHandle`).
pub trait SecureKeystoreExt<R: Runtime> {
    /// Obtain the secure-keystore handle. Always present (the plugin is
    /// registered on every target); on non-Android targets the handle is inert.
    fn secure_keystore(&self) -> &SecureKeystore<R>;
}

impl<R: Runtime, T: Manager<R>> SecureKeystoreExt<R> for T {
    fn secure_keystore(&self) -> &SecureKeystore<R> {
        self.state::<SecureKeystore<R>>().inner()
    }
}

// ---------------------------------------------------------------------------
// Plugin initialization
// ---------------------------------------------------------------------------

/// Initializes the secure-keystore plugin.
///
/// On Android, registers the Kotlin `SecureKeystorePlugin` class and manages
/// the handle. On desktop, manages an inert stub so
/// [`SecureKeystoreExt::secure_keystore`] is always callable.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("secure-keystore")
        .setup(|app, #[allow(unused_variables)] api| {
            #[cfg(target_os = "android")]
            {
                let handle =
                    api.register_android_plugin(PLUGIN_IDENTIFIER, "SecureKeystorePlugin")?;
                app.manage(SecureKeystore(handle));
            }
            #[cfg(not(target_os = "android"))]
            {
                app.manage(SecureKeystore::<R>(std::marker::PhantomData));
            }
            Ok(())
        })
        .build()
}
