// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! gpm-specific Keystore configuration: the alias/prefs names, key policies,
//! and brand fallbacks that the generic keystore plugins carry **no** knowledge
//! of. This is the single source of truth for gpm's Keystore identifiers — the
//! plugins are called with these as plain parameters, so the plugin crates stay
//! app-agnostic and publishable. When the two keystore plugins merge, only this
//! module's handle targets change; the business call-sites (applock / biometric
//! / m0007) stay put.

use tauri::Runtime;
use tauri_plugin_biometric_keystore::{
    KeyPolicy, PromptText, ResolvedPromptText, resolve_prompt_text,
};
use tauri_plugin_secure_keystore::{AliasState, SecureKeystore, SecureKeystoreError};

// ---------------------------------------------------------------------------
// Identity passphrase slot (biometric-gated, biometric-keystore plugin)
// ---------------------------------------------------------------------------

/// Keystore alias for the sealed identity passphrase.
pub(crate) const PASSPHRASE_ALIAS: &str = "gpm_passphrase";
/// `SharedPreferences` file holding the sealed passphrase ciphertext.
pub(crate) const PASSPHRASE_PREFS: &str = "gpm_keystore";

/// Key policy for the identity passphrase: biometric-gated (auth + STRONG),
/// **invalidated by biometric enrollment**. A passphrase can be re-entered, so
/// a fingerprint change correctly forces re-enabling (the self-heal path in
/// `biometric_unlock`). Per-use auth (`auth_validity_seconds = 0`).
pub(crate) const PASSPHRASE_POLICY: KeyPolicy = KeyPolicy {
    auth_required: true,
    auth_biometric_strong: true,
    invalidated_by_enrollment: true,
    auth_validity_seconds: 0,
};

/// gpm brand fallback for the biometric prompt title (the app name). The plugin
/// carries no brand string — it surfaces only caller-supplied text.
pub(crate) const PROMPT_FALLBACK_TITLE: &str = "gpm";
/// gpm brand fallback for the biometric prompt negative (cancel) button.
pub(crate) const PROMPT_FALLBACK_NEGATIVE: &str = "Cancel";

/// Resolve a frontend-supplied [`PromptText`] against gpm's brand fallbacks.
/// `None` (frontend omitted it) resolves to the bare fallbacks.
pub(crate) fn resolve_prompt(prompt: Option<&PromptText>) -> ResolvedPromptText {
    let empty = PromptText {
        title: None,
        subtitle: None,
        negative: None,
    };
    resolve_prompt_text(
        prompt.unwrap_or(&empty),
        PROMPT_FALLBACK_TITLE,
        PROMPT_FALLBACK_NEGATIVE,
    )
}

// ---------------------------------------------------------------------------
// At-rest master key + biometric vault/legacy slots (secure-keystore plugin)
// ---------------------------------------------------------------------------
//
// R064 split the at-rest seal into two keys, both stored via the
// (now generic) secure-keystore plugin:
// - the **auth-free master key** (permanent; seals `repo.json` + `app.json`;
//   the headless worker reads it under lock) — `MASTER_*` below.
// - the **biometric-gated vault key** (opt-in App Lock; seals `identity` +
//   `app_id_pass`) and its pre-R064 **legacy** alias (m0007 relocates legacy →
//   master and mints the vault) — `BiometricSlot` below.
// The plugin carries none of these identifiers; they live here and are passed
// as plain alias/prefs/policy parameters.

/// Keystore alias for the auth-free at-rest master key.
pub(crate) const MASTER_ALIAS: &str = "gpm_master_key";
/// `SharedPreferences` file holding the sealed master-key ciphertext.
pub(crate) const MASTER_PREFS: &str = "gpm_secure_keystore";

/// Key policy for the auth-free master key: no auth, no enrollment invalidation
/// — the at-rest store never bricks on a fingerprint change, and the headless
/// worker can read `repo.json` under lock. Per R064 this key is permanent.
pub(crate) const MASTER_FREE_POLICY: tauri_plugin_secure_keystore::KeyPolicy =
    tauri_plugin_secure_keystore::KeyPolicy {
        auth_required: false,
        auth_biometric_strong: false,
        invalidated_by_enrollment: false,
        auth_validity_seconds: 0,
    };

/// Key policy for the biometric-gated slots (vault + legacy): per-use STRONG
/// biometric auth, but **not** invalidated by enrollment — adding a fingerprint
/// must not brick the store (the master key cannot self-heal the way a
/// passphrase can). Removing ALL biometrics still invalidates the key → the
/// documented re-setup case. Same policy for both slots (R064).
pub(crate) const VAULT_POLICY: tauri_plugin_secure_keystore::KeyPolicy =
    tauri_plugin_secure_keystore::KeyPolicy {
        auth_required: true,
        auth_biometric_strong: true,
        invalidated_by_enrollment: false,
        auth_validity_seconds: 0,
    };

/// Which biometric-gated Keystore alias an app-lock command targets (R064
/// master/vault split). Moved here from the plugin crate so the plugin stays
/// app-agnostic.
///
/// - [`BiometricSlot::Vault`] — `gpm_vault_key`: seals `identity` + `app_id_pass`
///   when App Lock is ON.
/// - [`BiometricSlot::Legacy`] — `gpm_master_key_biometric`: held the master
///   under App Lock pre-R064; m0007 relocates it to the auth-free master and
///   deletes this alias.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BiometricSlot {
    /// `gpm_vault_key` — the distinct biometric vault key (R064).
    Vault,
    /// `gpm_master_key_biometric` — the pre-R064 master-key alias, relocated to
    /// the auth-free store and deleted by m0007.
    Legacy,
}

impl BiometricSlot {
    /// The Keystore alias for this slot.
    #[must_use]
    pub(crate) const fn alias(self) -> &'static str {
        match self {
            Self::Vault => "gpm_vault_key",
            Self::Legacy => "gpm_master_key_biometric",
        }
    }

    /// The `SharedPreferences` file holding this slot's ciphertext.
    #[must_use]
    pub(crate) const fn prefs(self) -> &'static str {
        match self {
            Self::Vault => "gpm_secure_keystoreVault",
            Self::Legacy => "gpm_secure_keystoreBiometric",
        }
    }
}

/// `BIOMETRIC_NOT_SET` / `SECURE_KEYSTORE_UNAVAILABLE` both mean "nothing usable
/// here" for a speculative read (first run, desktop, or a cleared slot).
fn is_not_set_or_unavailable(e: &SecureKeystoreError) -> bool {
    e.code == "BIOMETRIC_NOT_SET" || e.code == "SECURE_KEYSTORE_UNAVAILABLE"
}

/// Resolve an app-lock prompt against gpm's brand fallbacks (the same "gpm" /
/// "Cancel" the biometric-unlock prompt uses).
pub(crate) fn resolve_app_lock_prompt(
    prompt: Option<&tauri_plugin_secure_keystore::PromptText>,
) -> tauri_plugin_secure_keystore::ResolvedPromptText {
    let empty = tauri_plugin_secure_keystore::PromptText {
        title: None,
        subtitle: None,
        negative: None,
    };
    tauri_plugin_secure_keystore::resolve_prompt_text(
        prompt.unwrap_or(&empty),
        PROMPT_FALLBACK_TITLE,
        PROMPT_FALLBACK_NEGATIVE,
    )
}

/// Retrieve the auth-free master key (Base64), or `None` if not provisioned /
/// desktop. Non-prompting.
pub(crate) async fn retrieve_master<R: Runtime>(
    ks: &SecureKeystore<R>,
) -> Result<Option<String>, SecureKeystoreError> {
    match ks
        .retrieve(MASTER_ALIAS, MASTER_PREFS, MASTER_FREE_POLICY, None)
        .await
    {
        Ok(b) => Ok(Some(b)),
        Err(e) if is_not_set_or_unavailable(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Seal the auth-free master key (Base64) into the Keystore. Non-prompting.
pub(crate) async fn store_master<R: Runtime>(
    ks: &SecureKeystore<R>,
    key_b64: &str,
) -> Result<(), SecureKeystoreError> {
    ks.store(
        key_b64,
        MASTER_ALIAS,
        MASTER_PREFS,
        MASTER_FREE_POLICY,
        None,
    )
    .await
}

/// Retrieve a biometric-gated slot key (Base64), or `None` if nothing is sealed.
/// **Shows a `BiometricPrompt`** (DECRYPT) only when something IS sealed — a
/// missing slot returns `None` with no prompt.
pub(crate) async fn retrieve_slot<R: Runtime>(
    ks: &SecureKeystore<R>,
    slot: BiometricSlot,
    prompt: Option<&tauri_plugin_secure_keystore::PromptText>,
) -> Result<Option<String>, SecureKeystoreError> {
    let resolved = resolve_app_lock_prompt(prompt);
    match ks
        .retrieve(slot.alias(), slot.prefs(), VAULT_POLICY, Some(&resolved))
        .await
    {
        Ok(b) => Ok(Some(b)),
        Err(e) if is_not_set_or_unavailable(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Seal a value (Base64) into a biometric-gated slot. **Shows a `BiometricPrompt`**
/// (ENCRYPT). `slot` selects vault vs legacy.
pub(crate) async fn store_slot<R: Runtime>(
    ks: &SecureKeystore<R>,
    value: &str,
    slot: BiometricSlot,
    prompt: Option<&tauri_plugin_secure_keystore::PromptText>,
) -> Result<(), SecureKeystoreError> {
    let resolved = resolve_app_lock_prompt(prompt);
    ks.store(
        value,
        slot.alias(),
        slot.prefs(),
        VAULT_POLICY,
        Some(&resolved),
    )
    .await
}

/// Delete a biometric-gated slot's Keystore key + ciphertext (best-effort).
pub(crate) async fn delete_slot<R: Runtime>(
    ks: &SecureKeystore<R>,
    slot: BiometricSlot,
) -> Result<(), SecureKeystoreError> {
    ks.delete(slot.alias(), slot.prefs()).await
}

/// Whether the app-launch biometric gate is armed: a usable key exists in EITHER
/// the vault or the legacy biometric slot (R064 dual-alias; covers the m0007
/// transition — pre-m0007 upgraders hold the legacy alias, post-m0007 the
/// vault). Two non-prompting `alias_state` probes, OR'd in Rust (the plugin no
/// longer carries the slot OR). Sequential: each probe is a fast cipher-init
/// check (<5 ms, no prompt), run once at cold start.
pub(crate) async fn has_app_lock_enabled<R: Runtime>(ks: &SecureKeystore<R>) -> bool {
    let ok = |r: Result<AliasState, SecureKeystoreError>| r.is_ok_and(|s| s.present && s.usable);
    // Short-circuit on the common post-m0007 case (vault present) before probing
    // legacy; legacy is only set on the pre-m0007 upgrader path.
    ok(ks
        .alias_state(BiometricSlot::Vault.alias(), BiometricSlot::Vault.prefs())
        .await)
        || ok(ks
            .alias_state(BiometricSlot::Legacy.alias(), BiometricSlot::Legacy.prefs())
            .await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn biometric_slot_alias_and_prefs() {
        assert_eq!(BiometricSlot::Vault.alias(), "gpm_vault_key");
        assert_eq!(BiometricSlot::Vault.prefs(), "gpm_secure_keystoreVault");
        assert_eq!(BiometricSlot::Legacy.alias(), "gpm_master_key_biometric");
        assert_eq!(
            BiometricSlot::Legacy.prefs(),
            "gpm_secure_keystoreBiometric"
        );
    }

    #[test]
    fn master_free_policy_is_auth_free() {
        // Pin the auth-free master policy: no auth, no enrollment invalidation.
        let p = MASTER_FREE_POLICY;
        assert!(!p.auth_required);
        assert!(!p.invalidated_by_enrollment);
    }

    #[test]
    fn vault_policy_is_biometric_gated_and_enrollment_surviving() {
        // R064: per-use STRONG biometric, but survives fingerprint enrollment.
        let p = VAULT_POLICY;
        assert!(p.auth_required);
        assert!(p.auth_biometric_strong);
        assert!(!p.invalidated_by_enrollment);
    }

    #[test]
    fn resolve_app_lock_prompt_uses_brand_fallback() {
        let r = resolve_app_lock_prompt(None);
        assert_eq!(r.title, "gpm");
        assert_eq!(r.negative, "Cancel");
        assert!(r.subtitle.is_none());
    }
}
