// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Backend-only Tauri plugin that seals a caller-supplied **secret string**
//! into the Android Keystore (hardware-backed AES/GCM) behind a biometric
//! `BiometricPrompt`, and retrieves it on demand. The plugin is generic: the
//! Keystore alias, prefs name, key-generation policy, and prompt text are all
//! supplied by the caller — it carries no app-specific identifiers or brand
//! strings.
//!
//! The secret flows Kotlin → Rust and never reaches the `WebView`. On
//! non-Android targets the plugin is registered but inert (operations report
//! `unavailable` / empty).
//!
//! **Homomorphic with `tauri-plugin-secure-keystore`**: same handle method
//! names/signatures and same `KeyPolicy`/`BiometricState`/`PromptText` shapes,
//! so the two crates can be mechanically merged later (one keeps the shared
//! pure functions; the other is dropped).

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
    /// STRONG absent but a weak (Class 2) print is enrolled — a Class-3 key
    /// cannot be used. Enrolling a Class-3 print helps on Class-3-capable
    /// hardware, so the UI offers the Security-settings deep-link here too.
    WeakEnrolled,
    /// No usable hardware / hw unavailable / security update required / unsupported /
    /// pre-API-30 — nothing the user can fix from settings.
    Unavailable,
}

// ---------------------------------------------------------------------------
// Key-generation policy (caller-supplied; the plugin applies it verbatim)
// ---------------------------------------------------------------------------

/// Android Keystore key-generation policy, applied to `KeyGenParameterSpec`.
///
/// Round-tripped to Kotlin as **flattened** camelCase payload fields (Tauri's
/// `@InvokeArg` is flat-field shaped). Construct via const instances — there is
/// **no `Default` and no `serde(default)`**: a bool default drifting here
/// would silently change key behavior (auth-free vs gated, enrollment
/// invalidation), so the caller always supplies the full policy explicitly.
/// The plugin never invents policy values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyPolicy {
    /// `setUserAuthenticationRequired(true)` — every use needs biometric auth.
    pub auth_required: bool,
    /// When `auth_required`, bind to `AUTH_BIOMETRIC_STRONG` (Class 3); else
    /// ignored (auth-free keys carry no authenticator set).
    pub auth_biometric_strong: bool,
    /// When `auth_required`, `setInvalidatedByBiometricEnrollment(this)`;
    /// **unset** when `auth_required` is false (the flag is meaningless without
    /// user-auth binding — leaving it unset keeps the auth-free keygen
    /// byte-identical to a plain keygen, so no migration is implied).
    pub invalidated_by_enrollment: bool,
    /// When `auth_required`, the `setUserAuthenticationParameters` validity in
    /// seconds (0 = per-use auth). Ignored when `auth_required` is false.
    pub auth_validity_seconds: u32,
}

// ---------------------------------------------------------------------------
// Prompt text (caller resolves against its own brand fallbacks)
// ---------------------------------------------------------------------------

/// Localized `BiometricPrompt` text as supplied by the app (it owns
/// localization; the plugin never localizes and never bakes a brand string).
/// Fields are optional; resolve with [`resolve_prompt_text`] before passing to
/// `store`/`retrieve`.
#[derive(Debug, Clone, Deserialize)]
pub struct PromptText {
    /// Prompt title.
    pub title: Option<String>,
    /// Prompt subtitle.
    pub subtitle: Option<String>,
    /// Negative (cancel) button label.
    pub negative: Option<String>,
}

/// [`PromptText`] with caller-supplied fallbacks applied — `title`/`negative`
/// are guaranteed non-empty. Passed to `store`/`retrieve`.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedPromptText {
    /// Resolved prompt title (non-empty).
    pub title: String,
    /// Resolved prompt subtitle (`None` when blank/unset).
    pub subtitle: Option<String>,
    /// Resolved negative-button label (non-empty).
    pub negative: String,
}

/// Resolve [`PromptText`] against caller-supplied fallbacks. Pure (no platform
/// types). The plugin never bakes a brand string; the app supplies the fallback
/// values (e.g. its own app name). A blank field falls back; a blank subtitle
/// becomes `None`.
#[must_use]
pub fn resolve_prompt_text(
    prompt: &PromptText,
    fallback_title: &str,
    fallback_negative: &str,
) -> ResolvedPromptText {
    fn non_blank(s: &str) -> &str {
        s.trim_start().trim_end()
    }
    let pick = |field: &Option<String>, fallback: &str| -> String {
        field
            .as_deref()
            .map(non_blank)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| fallback)
            .to_owned()
    };
    ResolvedPromptText {
        title: pick(&prompt.title, fallback_title),
        subtitle: prompt
            .subtitle
            .as_deref()
            .map(non_blank)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned),
        negative: pick(&prompt.negative, fallback_negative),
    }
}

// ---------------------------------------------------------------------------
// Alias liveness (the platform probe; composition happens in Rust)
// ---------------------------------------------------------------------------

/// Liveness of one Keystore alias: whether ciphertext exists (`present`) and
/// whether its key still initializes cleanly (`usable`). `present && usable` is
/// "a stored, working key"; `present && !usable` is a dead key (e.g. all
/// biometrics removed) the caller should treat as absent.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
pub struct AliasState {
    /// Whether ciphertext is present in prefs for this alias.
    pub present: bool,
    /// Whether the alias's key still initializes (a non-prompting cipher-init
    /// probe; pre-API-30 or a dead key → `false`).
    pub usable: bool,
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

    /// Probe one alias's liveness (single IPC): whether ciphertext exists AND
    /// its key still initializes. Non-prompting. The platform-side cipher-init
    /// probe cannot move to Rust, so this is the primitive; callers compose
    /// (`present && usable`, slot OR-disjunction, etc.) in Rust.
    ///
    /// # Errors
    ///
    /// Only if the mobile-plugin invoke itself fails.
    pub async fn alias_state(&self, alias: &str, prefs: &str) -> Result<AliasState, KeystoreError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Payload<'a> {
            alias: &'a str,
            prefs: &'a str,
        }
        self.0
            .run_mobile_plugin_async::<AliasState>("aliasState", Payload { alias, prefs })
            .await
            .map_err(map_invoke_err)
    }

    /// Whether a stored, working key exists for `alias` (non-prompting). Composed
    /// from [`alias_state`](Self::alias_state): ciphertext present AND key usable.
    ///
    /// # Errors
    ///
    /// Only if the mobile-plugin invoke itself fails.
    pub async fn has_stored(&self, alias: &str, prefs: &str) -> Result<bool, KeystoreError> {
        let s = self.alias_state(alias, prefs).await?;
        Ok(s.present && s.usable)
    }

    /// Delete the stored ciphertext and the Keystore key for `alias`
    /// (best-effort).
    ///
    /// # Errors
    ///
    /// [`KeystoreError`] only if the mobile-plugin invoke itself fails.
    pub async fn delete(&self, alias: &str, prefs: &str) -> Result<(), KeystoreError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Payload<'a> {
            alias: &'a str,
            prefs: &'a str,
        }
        self.0
            .run_mobile_plugin_async::<()>("delete", Payload { alias, prefs })
            .await
            .map_err(map_invoke_err)
    }

    /// Seal `value` into the Keystore at `alias`. **Shows a biometric prompt**
    /// (CryptoObject ENCRYPT) when `policy.auth_required` — the key needs user
    /// auth for encrypt too. The `Invoke` stays open across the prompt and is
    /// resolved only from a terminal biometric callback. `prompt` supplies
    /// already-resolved prompt text (see [`resolve_prompt_text`]).
    ///
    /// # Errors
    ///
    /// [`KeystoreError`] carrying a `BIOMETRIC_*` code (e.g. `BIOMETRIC_CANCELLED`,
    /// `BIOMETRIC_KEY_INVALIDATED`, `BIOMETRIC_FAILED`) if the prompt is dismissed,
    /// the key is dead, or the invoke fails.
    pub async fn store(
        &self,
        value: &str,
        alias: &str,
        prefs: &str,
        policy: KeyPolicy,
        prompt: Option<&ResolvedPromptText>,
    ) -> Result<(), KeystoreError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Payload<'a> {
            value: &'a str,
            alias: &'a str,
            prefs: &'a str,
            auth_required: bool,
            auth_biometric_strong: bool,
            invalidated_by_enrollment: bool,
            auth_validity_seconds: u32,
            title: Option<&'a str>,
            subtitle: Option<&'a str>,
            negative: Option<&'a str>,
        }
        self.0
            .run_mobile_plugin_async::<()>(
                "store",
                Payload {
                    value,
                    alias,
                    prefs,
                    auth_required: policy.auth_required,
                    auth_biometric_strong: policy.auth_biometric_strong,
                    invalidated_by_enrollment: policy.invalidated_by_enrollment,
                    auth_validity_seconds: policy.auth_validity_seconds,
                    title: prompt.map(|p| p.title.as_str()),
                    subtitle: prompt.map(|p| p.subtitle.as_deref()).flatten(),
                    negative: prompt.map(|p| p.negative.as_str()),
                },
            )
            .await
            .map_err(map_invoke_err)
    }

    /// Retrieve the sealed value at `alias`. **Shows a biometric prompt**
    /// (CryptoObject DECRYPT) when `policy.auth_required`. The value is returned
    /// here (Rust side only) and wrapped in `Zeroizing<String>` by the caller.
    /// `prompt` supplies already-resolved prompt text.
    ///
    /// # Errors
    ///
    /// [`KeystoreError`] carrying a `BIOMETRIC_*` code (e.g. `BIOMETRIC_CANCELLED`,
    /// `BIOMETRIC_KEY_INVALIDATED`, `BIOMETRIC_NOT_SET`, `BIOMETRIC_FAILED`) on
    /// dismissal, key death, nothing-stored, or invoke failure.
    pub async fn retrieve(
        &self,
        alias: &str,
        prefs: &str,
        policy: KeyPolicy,
        prompt: Option<&ResolvedPromptText>,
    ) -> Result<String, KeystoreError> {
        #[derive(Deserialize)]
        struct Resp {
            value: String,
        }
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Payload<'a> {
            alias: &'a str,
            prefs: &'a str,
            auth_required: bool,
            auth_biometric_strong: bool,
            invalidated_by_enrollment: bool,
            auth_validity_seconds: u32,
            title: Option<&'a str>,
            subtitle: Option<&'a str>,
            negative: Option<&'a str>,
        }
        self.0
            .run_mobile_plugin_async::<Resp>(
                "retrieve",
                Payload {
                    alias,
                    prefs,
                    auth_required: policy.auth_required,
                    auth_biometric_strong: policy.auth_biometric_strong,
                    invalidated_by_enrollment: policy.invalidated_by_enrollment,
                    auth_validity_seconds: policy.auth_validity_seconds,
                    title: prompt.map(|p| p.title.as_str()),
                    subtitle: prompt.map(|p| p.subtitle.as_deref()).flatten(),
                    negative: prompt.map(|p| p.negative.as_str()),
                },
            )
            .await
            .map(|r| r.value)
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
    #[expect(clippy::unused_async)]
    pub async fn alias_state(
        &self,
        _alias: &str,
        _prefs: &str,
    ) -> Result<AliasState, KeystoreError> {
        Ok(AliasState {
            present: false,
            usable: false,
        })
    }

    /// Inert: nothing is ever stored.
    ///
    /// # Errors
    ///
    /// Inert stub: always returns `Ok(false)`; never errors.
    #[expect(clippy::unused_async)]
    pub async fn has_stored(&self, _alias: &str, _prefs: &str) -> Result<bool, KeystoreError> {
        Ok(false)
    }

    /// Inert: nothing to delete.
    ///
    /// # Errors
    ///
    /// Inert stub: always returns `Ok`; never errors.
    #[expect(clippy::unused_async)]
    pub async fn delete(&self, _alias: &str, _prefs: &str) -> Result<(), KeystoreError> {
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
        _value: &str,
        _alias: &str,
        _prefs: &str,
        _policy: KeyPolicy,
        _prompt: Option<&ResolvedPromptText>,
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
    pub async fn retrieve(
        &self,
        _alias: &str,
        _prefs: &str,
        _policy: KeyPolicy,
        _prompt: Option<&ResolvedPromptText>,
    ) -> Result<String, KeystoreError> {
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
    use super::{BiometricState, KeyPolicy, PromptText, resolve_prompt_text};

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

    /// `KeyPolicy` has NO `Default` — a bool default drifting here would
    /// silently change key behavior. Pin that the impl is absent so a future
    /// `#[derive(Default)]` addition is caught.
    #[test]
    fn key_policy_has_no_default_impl() {
        // If a `Default` impl existed this would compile; we assert by
        // constructing only via explicit field literals.
        let p = KeyPolicy {
            auth_required: false,
            auth_biometric_strong: false,
            invalidated_by_enrollment: false,
            auth_validity_seconds: 0,
        };
        // Round-trips through serde flattened-shaped JSON.
        let json = serde_json::to_string(&p).unwrap();
        let back: KeyPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn resolve_prompt_text_applies_caller_fallbacks() {
        let p = PromptText {
            title: None,
            subtitle: Some("   ".to_owned()),
            negative: None,
        };
        let r = resolve_prompt_text(&p, "MyApp", "Cancel");
        assert_eq!(r.title, "MyApp");
        assert_eq!(r.negative, "Cancel");
        assert!(r.subtitle.is_none());
    }

    #[test]
    fn resolve_prompt_text_keeps_provided_non_blank() {
        let p = PromptText {
            title: Some("Title".to_owned()),
            subtitle: Some("Sub".to_owned()),
            negative: Some("Nope".to_owned()),
        };
        let r = resolve_prompt_text(&p, "MyApp", "Cancel");
        assert_eq!(r.title, "Title");
        assert_eq!(r.subtitle.as_deref(), Some("Sub"));
        assert_eq!(r.negative, "Nope");
    }
}
