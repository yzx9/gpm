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
//! the `WebView`.
//!
//! On desktop (and any non-Android target) the plugin is registered but inert:
//! every operation reports [`KeystoreError::unavailable`], so
//! `is_available`/`has_stored` read `false` and the UI falls back to the
//! passphrase form.

#[cfg(not(target_os = "android"))]
use std::marker::PhantomData;

use serde::{Deserialize, Serialize};
#[cfg(target_os = "android")]
use tauri::plugin::mobile::PluginInvokeError;
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
fn map_invoke_err(err: PluginInvokeError) -> KeystoreError {
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
// Availability state (quad-state: STRONG + WEAK canAuthenticate mapping)
// ---------------------------------------------------------------------------

/// Biometric availability state reported by the Kotlin plugin. Mirrors the
/// strings emitted by `mapBiometricState` (KeystorePlugin.kt) via
/// `#[serde(rename_all = "snake_case")]`; the cross-layer string contract
/// (Kotlin emitter ↔ Rust deserializer ↔ TS union) is pinned by tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BiometricState {
    /// API 30+ with a STRONG (Class 3) biometric enrolled — biometric unlock usable.
    Available,
    /// STRONG biometric absent AND nothing enrolled — the actionable "go enroll" case.
    NoEnrollment,
    /// STRONG absent but a weak (Class 2) print is enrolled — gpm needs Class 3.
    /// Enrolling a Class-3 print helps on Class-3-capable hardware, so the UI
    /// offers the Security-settings deep-link here too.
    WeakEnrolled,
    /// No usable hardware / hw unavailable / security update required / unsupported /
    /// pre-API-30 — nothing the user can fix from settings.
    Unavailable,
}

// ---------------------------------------------------------------------------
// Prompt text
// ---------------------------------------------------------------------------

/// Localized `BiometricPrompt` text supplied by the frontend, so the native
/// layer never localizes. Deserialized from the `{ title, subtitle, negative }`
/// shape the `WebView` sends and forwarded to Kotlin, which falls back to a
/// generic safety string when a field is absent (a missing field never bricks
/// the prompt). Defined here (not the app crate) so the app command's IPC param
/// type IS the plugin's type — no cross-crate struct sharing.
#[derive(Debug, Clone, Deserialize)]
pub struct PromptText {
    /// Prompt title.
    pub title: Option<String>,
    /// Prompt subtitle.
    pub subtitle: Option<String>,
    /// Negative (cancel) button label.
    pub negative: Option<String>,
}

// ---------------------------------------------------------------------------
// Keystore handle (cfg-gated: real on Android, stub elsewhere)
// ---------------------------------------------------------------------------

/// Handle to the keystore. On Android it wraps the mobile plugin handle; on
/// other targets it is an inert stub whose operations report unavailable.
#[cfg(target_os = "android")]
#[derive(Debug)]
pub struct Keystore<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// Handle to the keystore — inert stub on non-Android targets.
///
/// `PhantomData<fn() -> R>` keeps the stub `Send + Sync` unconditionally (the
/// `fn() -> R` variance does not inherit R's auto-trait bounds), so it can be
/// managed as app state on every target.
#[cfg(not(target_os = "android"))]
#[derive(Debug)]
pub struct Keystore<R: Runtime>(PhantomData<fn() -> R>);

#[cfg(target_os = "android")]
impl<R: Runtime> Keystore<R> {
    /// Whether biometric-gated storage is usable on this device, as a quad-state
    /// ([`BiometricState`]). Fast / non-prompting. Pre-API-30 → `Unavailable`.
    ///
    /// # Errors
    ///
    /// Only if the mobile-plugin invoke itself fails.
    pub async fn is_available(&self) -> Result<BiometricState, KeystoreError> {
        #[derive(Deserialize)]
        struct Resp {
            state: BiometricState,
        }
        self.0
            .run_mobile_plugin_async::<Resp>("isAvailable", ())
            .await
            .map(|r| r.state)
            .map_err(map_invoke_err)
    }

    /// Open the system Security settings (the biometric-enrollment surface) — the
    /// recovery target when [`is_available`](Self::is_available) reports
    /// `NoEnrollment`. Returns whether a handler activity was found (`false` on
    /// the rare OEM ROM lacking the target) so the caller can toast instead of
    /// failing silently.
    pub async fn open_security_settings(&self) -> bool {
        #[derive(Deserialize)]
        struct Resp {
            opened: bool,
        }
        self.0
            .run_mobile_plugin_async::<Resp>("openSecuritySettings", ())
            .await
            .map(|r| r.opened)
            .unwrap_or_else(|e| {
                // `opened: false` from the Kotlin catch (no handler activity) is
                // expected; a plugin-invoke failure here is not, so log it before
                // collapsing to false — otherwise the recovery tap fails silently.
                log::warn!("open_security_settings: plugin invoke failed: {e:?}");
                false
            })
    }

    /// Whether a stored passphrase exists (non-prompting read of the
    /// ciphertext state in prefs).
    ///
    /// # Errors
    ///
    /// Only if the mobile-plugin invoke itself fails.
    pub async fn has_stored(&self) -> Result<bool, KeystoreError> {
        #[derive(Deserialize)]
        struct Resp {
            stored: bool,
        }
        self.0
            .run_mobile_plugin_async::<Resp>("hasStored", ())
            .await
            .map(|r| r.stored)
            .map_err(map_invoke_err)
    }

    /// Delete the stored passphrase and the Keystore key (best-effort).
    ///
    /// # Errors
    ///
    /// [`KeystoreError`] only if the mobile-plugin invoke itself fails.
    pub async fn delete(&self) -> Result<(), KeystoreError> {
        self.0
            .run_mobile_plugin_async::<()>("delete", ())
            .await
            .map_err(map_invoke_err)
    }

    /// Seal `passphrase` into the Keystore. **Shows a biometric prompt**
    /// (CryptoObject ENCRYPT) — the key is `setUserAuthenticationRequired`,
    /// so encrypt needs user auth too. The `Invoke` stays open across the
    /// prompt and is resolved only from a terminal biometric callback. `prompt`
    /// supplies the localized prompt text.
    ///
    /// # Errors
    ///
    /// [`KeystoreError`] carrying a `BIOMETRIC_*` code (e.g. `BIOMETRIC_CANCELLED`,
    /// `BIOMETRIC_KEY_INVALIDATED`, `BIOMETRIC_FAILED`) if the prompt is dismissed,
    /// the key is dead, or the invoke fails.
    pub async fn store(
        &self,
        passphrase: &str,
        prompt: Option<&PromptText>,
    ) -> Result<(), KeystoreError> {
        #[derive(Serialize)]
        struct Payload<'a> {
            passphrase: &'a str,
            title: Option<&'a str>,
            subtitle: Option<&'a str>,
            negative: Option<&'a str>,
        }
        self.0
            .run_mobile_plugin_async::<()>(
                "store",
                Payload {
                    passphrase,
                    title: prompt.and_then(|p| p.title.as_deref()),
                    subtitle: prompt.and_then(|p| p.subtitle.as_deref()),
                    negative: prompt.and_then(|p| p.negative.as_deref()),
                },
            )
            .await
            .map_err(map_invoke_err)
    }

    /// Retrieve the sealed passphrase. **Shows a biometric prompt**
    /// (CryptoObject DECRYPT). The passphrase is returned here (Rust side
    /// only) and wrapped in `Zeroizing<String>` by the caller. `prompt` supplies
    /// the localized prompt text.
    ///
    /// # Errors
    ///
    /// [`KeystoreError`] carrying a `BIOMETRIC_*` code (e.g. `BIOMETRIC_CANCELLED`,
    /// `BIOMETRIC_KEY_INVALIDATED`, `BIOMETRIC_FAILED`) on dismissal, key death,
    /// or invoke failure.
    pub async fn retrieve(&self, prompt: Option<&PromptText>) -> Result<String, KeystoreError> {
        #[derive(Deserialize)]
        struct Resp {
            passphrase: String,
        }
        #[derive(Serialize)]
        struct Payload<'a> {
            title: Option<&'a str>,
            subtitle: Option<&'a str>,
            negative: Option<&'a str>,
        }
        self.0
            .run_mobile_plugin_async::<Resp>(
                "retrieve",
                Payload {
                    title: prompt.and_then(|p| p.title.as_deref()),
                    subtitle: prompt.and_then(|p| p.subtitle.as_deref()),
                    negative: prompt.and_then(|p| p.negative.as_deref()),
                },
            )
            .await
            .map(|r| r.passphrase)
            .map_err(map_invoke_err)
    }
}

#[cfg(not(target_os = "android"))]
impl<R: Runtime> Keystore<R> {
    /// Inert: biometric is never available on non-Android targets.
    ///
    /// # Errors
    ///
    /// Inert stub: always returns `Ok`; never errors.
    #[expect(clippy::unused_async)]
    pub async fn is_available(&self) -> Result<BiometricState, KeystoreError> {
        Ok(BiometricState::Unavailable)
    }

    /// Inert: nothing to open on desktop; reports `true` so a (never-shown on
    /// desktop) row never toasts a spurious failure.
    #[expect(clippy::unused_async)]
    pub async fn open_security_settings(&self) -> bool {
        true
    }

    /// Inert: nothing is ever stored.
    ///
    /// # Errors
    ///
    /// Inert stub: always returns `Ok(false)`; never errors.
    #[expect(clippy::unused_async)]
    pub async fn has_stored(&self) -> Result<bool, KeystoreError> {
        Ok(false)
    }

    /// Inert: nothing to delete.
    ///
    /// # Errors
    ///
    /// Inert stub: always returns `Ok`; never errors.
    #[expect(clippy::unused_async)]
    pub async fn delete(&self) -> Result<(), KeystoreError> {
        Ok(())
    }

    /// Inert: never succeeds — biometric is unavailable.
    ///
    /// # Errors
    ///
    /// Always returns [`KeystoreError::unavailable`] — biometric is unsupported
    /// off-Android.
    #[expect(clippy::unused_async)]
    pub async fn store(
        &self,
        _passphrase: &str,
        _prompt: Option<&PromptText>,
    ) -> Result<(), KeystoreError> {
        Err(KeystoreError::unavailable())
    }

    /// Inert: never succeeds — biometric is unavailable.
    ///
    /// # Errors
    ///
    /// Always returns [`KeystoreError::unavailable`] — biometric is unsupported
    /// off-Android.
    #[expect(clippy::unused_async)]
    pub async fn retrieve(&self, _prompt: Option<&PromptText>) -> Result<String, KeystoreError> {
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
#[must_use]
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
                app.manage(Keystore::<R>(PhantomData));
            }
            Ok(())
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::BiometricState;

    /// Pins the cross-layer contract: these exact `snake_case` strings are emitted
    /// by Kotlin's `mapBiometricState` and deserialized here.
    #[test]
    fn biometric_state_serializes_to_the_four_contract_strings() {
        assert_eq!(
            serde_json::to_string(&BiometricState::Available).unwrap(),
            "\"available\""
        );
        assert_eq!(
            serde_json::to_string(&BiometricState::NoEnrollment).unwrap(),
            "\"no_enrollment\""
        );
        assert_eq!(
            serde_json::to_string(&BiometricState::WeakEnrolled).unwrap(),
            "\"weak_enrolled\""
        );
        assert_eq!(
            serde_json::to_string(&BiometricState::Unavailable).unwrap(),
            "\"unavailable\""
        );
    }

    #[test]
    fn biometric_state_roundtrips_through_json() {
        for expected in [
            BiometricState::Available,
            BiometricState::NoEnrollment,
            BiometricState::WeakEnrolled,
            BiometricState::Unavailable,
        ] {
            let json = serde_json::to_string(&expected).unwrap();
            assert_eq!(
                serde_json::from_str::<BiometricState>(&json).unwrap(),
                expected
            );
        }
    }
}
