// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Tauri plugin that seals a caller-supplied **secret string** into the Android
//! Keystore (hardware-backed AES/GCM) under a caller-chosen policy — either
//! **auth-free** (no prompt, survives biometric changes) or **biometric-gated**
//! (a `BiometricPrompt` per use) — and retrieves it on demand. The plugin is
//! generic: the Keystore alias, prefs name, key-generation policy, and prompt
//! text are all supplied by the caller — it carries no app-specific identifiers
//! or brand strings.
//!
//! The secret flows Kotlin → Rust and never reaches the `WebView`. On
//! non-Android targets the plugin is registered but inert (operations report
//! `unavailable` / empty).
//!
//! **Homomorphic with `tauri-plugin-biometric-keystore`**: same handle method
//! names/signatures (`store`/`retrieve`/`delete`/`alias_state`/`has_stored`)
//! and same `KeyPolicy`/`BiometricState`/`PromptText` shapes, so the two crates
//! can be mechanically merged later (one keeps the shared pure functions; the
//! other is dropped). The one intentional divergence: this crate exposes
//! biometric availability as `is_biometric_available`, where
//! `biometric-keystore` exposes it as `is_available` — the merge picks one name.

#[cfg(not(target_os = "android"))]
use std::marker::PhantomData;

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
/// Carries the Kotlin `BIOMETRIC_*` / `SECURE_KEYSTORE_*` codes through to the
/// app layer. Serializes to `{ code, message }` and **never** contains secret
/// content — messages are derived only from exception class names or system
/// strings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecureKeystoreError {
    /// Machine-readable code, e.g. `SECURE_KEYSTORE_UNAVAILABLE`,
    /// `BIOMETRIC_NOT_SET`, `BIOMETRIC_KEY_INVALIDATED`, `BIOMETRIC_FAILED`.
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
// Availability state (quad-state: STRONG + WEAK canAuthenticate mapping)
// ---------------------------------------------------------------------------

/// Biometric availability state reported by the Kotlin plugin. Mirrors the
/// strings emitted by `mapBiometricState` (SecureKeystorePlugin.kt) and is
/// byte-identical to `tauri_plugin_biometric_keystore::BiometricState` — the two
/// plugins are separate crates so the type is duplicated; the cross-layer string
/// contract is pinned by tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BiometricState {
    /// API 30+ with a STRONG (Class 3) biometric enrolled.
    Available,
    /// STRONG absent, nothing enrolled.
    NoEnrollment,
    /// STRONG absent but a weak (Class 2) print is enrolled.
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
/// The plugin never invents policy values. Byte-identical to
/// `tauri_plugin_biometric_keystore::KeyPolicy` (the merge collapses the two).
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
    let pick = |field: &Option<String>, fallback: &str| -> String {
        field
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(fallback)
            .to_owned()
    };
    ResolvedPromptText {
        title: pick(&prompt.title, fallback_title),
        subtitle: prompt
            .subtitle
            .as_deref()
            .map(str::trim)
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

/// Handle to the secure keystore. On Android it wraps the mobile plugin handle;
/// on other targets it is an inert stub whose operations report unavailable.
#[cfg(target_os = "android")]
#[derive(Debug)]
pub struct SecureKeystore<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// Handle to the secure keystore — inert stub on non-Android targets.
///
/// `PhantomData<fn() -> R>` keeps the stub `Send + Sync` unconditionally (the
/// `fn() -> R` variance does not inherit R's auto-trait bounds), so it can be
/// managed as app state on every target.
#[cfg(not(target_os = "android"))]
#[derive(Debug)]
pub struct SecureKeystore<R: Runtime>(PhantomData<fn() -> R>);

#[cfg(target_os = "android")]
impl<R: Runtime> SecureKeystore<R> {
    /// Quad-state biometric availability ([`BiometricState`]). Fast / non-
    /// prompting. Pre-API-30 → `Unavailable`. The app-lock toggle is offered only
    /// on `Available`; callers derive `== Available` where they need a bool.
    ///
    /// # Errors
    ///
    /// Only if the mobile-plugin invoke itself fails.
    pub async fn is_biometric_available(&self) -> Result<BiometricState, SecureKeystoreError> {
        #[derive(Deserialize)]
        struct Resp {
            state: BiometricState,
        }
        self.0
            .run_mobile_plugin_async::<Resp>("isBiometricAvailable", ())
            .await
            .map(|r| r.state)
            .map_err(map_invoke_err)
    }

    /// Probe one alias's liveness (single IPC): whether ciphertext exists AND
    /// its key still initializes. Non-prompting. The platform-side cipher-init
    /// probe cannot move to Rust, so this is the primitive; callers compose
    /// (`present && usable`, slot OR-disjunction, etc.) in Rust.
    ///
    /// # Errors
    ///
    /// Only if the mobile-plugin invoke itself fails.
    pub async fn alias_state(
        &self,
        alias: &str,
        prefs: &str,
    ) -> Result<AliasState, SecureKeystoreError> {
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
    pub async fn has_stored(&self, alias: &str, prefs: &str) -> Result<bool, SecureKeystoreError> {
        let s = self.alias_state(alias, prefs).await?;
        Ok(s.present && s.usable)
    }

    /// Delete the stored ciphertext and the Keystore key for `alias`
    /// (best-effort).
    ///
    /// # Errors
    ///
    /// [`SecureKeystoreError`] only if the mobile-plugin invoke itself fails.
    pub async fn delete(&self, alias: &str, prefs: &str) -> Result<(), SecureKeystoreError> {
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
    /// resolved only from a terminal biometric callback. Auth-free policy seals
    /// directly with no prompt. `prompt` supplies already-resolved prompt text
    /// (see [`resolve_prompt_text`]).
    ///
    /// # Errors
    ///
    /// [`SecureKeystoreError`] carrying a `BIOMETRIC_*` / `SECURE_KEYSTORE_*`
    /// code (e.g. `BIOMETRIC_CANCELLED`, `BIOMETRIC_KEY_INVALIDATED`,
    /// `BIOMETRIC_FAILED`) if the prompt is dismissed, the key is dead, or the
    /// invoke fails.
    pub async fn store(
        &self,
        value: &str,
        alias: &str,
        prefs: &str,
        policy: KeyPolicy,
        prompt: Option<&ResolvedPromptText>,
    ) -> Result<(), SecureKeystoreError> {
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
    /// (CryptoObject DECRYPT) when `policy.auth_required`; an auth-free policy
    /// decrypts directly. The value is returned here (Rust side only) and wrapped
    /// in `Zeroizing<String>` by the caller. `prompt` supplies already-resolved
    /// prompt text.
    ///
    /// # Errors
    ///
    /// [`SecureKeystoreError`] carrying a `BIOMETRIC_*` / `SECURE_KEYSTORE_*`
    /// code: `BIOMETRIC_NOT_SET` when nothing is sealed (no prompt), and
    /// `BIOMETRIC_KEY_INVALIDATED` / `BIOMETRIC_CANCELLED` / `BIOMETRIC_FAILED`
    /// on dismissal, key death, or invoke failure.
    pub async fn retrieve(
        &self,
        alias: &str,
        prefs: &str,
        policy: KeyPolicy,
        prompt: Option<&ResolvedPromptText>,
    ) -> Result<String, SecureKeystoreError> {
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
impl<R: Runtime> SecureKeystore<R> {
    /// Inert: biometric is never available on non-Android targets.
    ///
    /// # Errors
    ///
    /// Inert stub: always returns `Ok`; never errors.
    #[expect(clippy::unused_async)]
    pub async fn is_biometric_available(&self) -> Result<BiometricState, SecureKeystoreError> {
        Ok(BiometricState::Unavailable)
    }

    /// Inert: nothing is ever stored.
    ///
    /// # Errors
    ///
    /// Inert stub: always returns `Ok`; never errors.
    #[expect(clippy::unused_async)]
    pub async fn alias_state(
        &self,
        _alias: &str,
        _prefs: &str,
    ) -> Result<AliasState, SecureKeystoreError> {
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
    pub async fn has_stored(
        &self,
        _alias: &str,
        _prefs: &str,
    ) -> Result<bool, SecureKeystoreError> {
        Ok(false)
    }

    /// Inert: nothing to delete.
    ///
    /// # Errors
    ///
    /// Inert stub: always returns `Ok`; never errors.
    #[expect(clippy::unused_async)]
    pub async fn delete(&self, _alias: &str, _prefs: &str) -> Result<(), SecureKeystoreError> {
        Ok(())
    }

    /// Inert: never succeeds — the secure keystore is unavailable.
    ///
    /// # Errors
    ///
    /// Always returns [`SecureKeystoreError::unavailable`] off-Android.
    #[expect(clippy::unused_async)]
    pub async fn store(
        &self,
        _value: &str,
        _alias: &str,
        _prefs: &str,
        _policy: KeyPolicy,
        _prompt: Option<&ResolvedPromptText>,
    ) -> Result<(), SecureKeystoreError> {
        Err(SecureKeystoreError::unavailable())
    }

    /// Inert: never succeeds — the secure keystore is unavailable.
    ///
    /// # Errors
    ///
    /// Always returns [`SecureKeystoreError::unavailable`] off-Android.
    #[expect(clippy::unused_async)]
    pub async fn retrieve(
        &self,
        _alias: &str,
        _prefs: &str,
        _policy: KeyPolicy,
        _prompt: Option<&ResolvedPromptText>,
    ) -> Result<String, SecureKeystoreError> {
        Err(SecureKeystoreError::unavailable())
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
#[must_use]
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
                app.manage(SecureKeystore::<R>(PhantomData));
            }
            Ok(())
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::{BiometricState, KeyPolicy, PromptText, resolve_prompt_text};

    /// Pins the cross-layer contract: byte-identical strings to
    /// `tauri_plugin_biometric_keystore::BiometricState` and to Kotlin's
    /// `mapBiometricState` (the type is duplicated across the two plugin
    /// crates; this test catches drift).
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

    /// `KeyPolicy` round-trips losslessly through serde (flattened camelCase) —
    /// the contract the IPC payload relies on. Uses a non-default value
    /// (`auth_required: true`) so a dropped/mis-serialized field changes the
    /// result. (The "no `Default`/no `serde(default)`" intent is a code
    /// convention on the type itself, not machine-enforced by this test.)
    #[test]
    fn key_policy_round_trips_through_serde() {
        let p = KeyPolicy {
            auth_required: true,
            auth_biometric_strong: true,
            invalidated_by_enrollment: false,
            auth_validity_seconds: 0,
        };
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
