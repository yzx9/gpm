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
//
// KNOWN DUPLICATE: `MasterKeyAccess.kt` in `tauri-plugin-background-sync`
// re-implements the auth-free master-key retrieve (the headless WorkManager
// worker has no `AppHandle`, so it cannot call the plugin's `@Command`). Its
// hardcoded `KEY_ALIAS`/`PREFS_NAME` must stay in sync with `MASTER_ALIAS`/
// `MASTER_PREFS` on rename. Promoting it to a shared Kotlin module is deferred
// — the cross-plugin Gradle dependency is unproven under Tauri's composite
// build, and the dedup is benign (a stable read-only decrypt path).

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

// ---------------------------------------------------------------------------
// Backend abstraction (mockable for tests)
// ---------------------------------------------------------------------------

/// The keystore backend the gpm app layer talks to. Abstracts the concrete
/// [`SecureKeystore<R>`] (a Tauri `PluginHandle` wrapper, not directly mockable)
/// so the app-layer helpers below are unit-testable with a recording mock —
/// pinning that each helper passes the correct alias/prefs/policy (the
/// regression guard for the caller-parametrized plugin refactor). The real impl
/// is a thin forward to the plugin's inherent methods.
pub(crate) trait KvKeystore {
    /// Probe one alias's liveness.
    async fn alias_state(
        &self,
        alias: &str,
        prefs: &str,
    ) -> Result<AliasState, SecureKeystoreError>;
    /// Delete an alias's key + ciphertext.
    async fn delete(&self, alias: &str, prefs: &str) -> Result<(), SecureKeystoreError>;
    /// Seal `value` at `alias` under `policy`.
    async fn store(
        &self,
        value: &str,
        alias: &str,
        prefs: &str,
        policy: tauri_plugin_secure_keystore::KeyPolicy,
        prompt: Option<&tauri_plugin_secure_keystore::ResolvedPromptText>,
    ) -> Result<(), SecureKeystoreError>;
    /// Retrieve the value sealed at `alias`.
    async fn retrieve(
        &self,
        alias: &str,
        prefs: &str,
        policy: tauri_plugin_secure_keystore::KeyPolicy,
        prompt: Option<&tauri_plugin_secure_keystore::ResolvedPromptText>,
    ) -> Result<String, SecureKeystoreError>;
}

impl<R: Runtime> KvKeystore for SecureKeystore<R> {
    async fn alias_state(
        &self,
        alias: &str,
        prefs: &str,
    ) -> Result<AliasState, SecureKeystoreError> {
        SecureKeystore::alias_state(self, alias, prefs).await
    }
    async fn delete(&self, alias: &str, prefs: &str) -> Result<(), SecureKeystoreError> {
        SecureKeystore::delete(self, alias, prefs).await
    }
    async fn store(
        &self,
        value: &str,
        alias: &str,
        prefs: &str,
        policy: tauri_plugin_secure_keystore::KeyPolicy,
        prompt: Option<&tauri_plugin_secure_keystore::ResolvedPromptText>,
    ) -> Result<(), SecureKeystoreError> {
        SecureKeystore::store(self, value, alias, prefs, policy, prompt).await
    }
    async fn retrieve(
        &self,
        alias: &str,
        prefs: &str,
        policy: tauri_plugin_secure_keystore::KeyPolicy,
        prompt: Option<&tauri_plugin_secure_keystore::ResolvedPromptText>,
    ) -> Result<String, SecureKeystoreError> {
        SecureKeystore::retrieve(self, alias, prefs, policy, prompt).await
    }
}

/// Retrieve the auth-free master key (Base64), or `None` if not provisioned /
/// desktop. Non-prompting.
pub(crate) async fn retrieve_master<K: KvKeystore>(
    ks: &K,
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
pub(crate) async fn store_master<K: KvKeystore>(
    ks: &K,
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
pub(crate) async fn retrieve_slot<K: KvKeystore>(
    ks: &K,
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
pub(crate) async fn store_slot<K: KvKeystore>(
    ks: &K,
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
pub(crate) async fn delete_slot<K: KvKeystore>(
    ks: &K,
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
pub(crate) async fn has_app_lock_enabled<K: KvKeystore>(ks: &K) -> bool {
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

    // ── Regression guard for the caller-parametrized plugin refactor ──────
    //
    // These pin that each app-layer helper passes the CORRECT alias/prefs/policy
    // to the backend — the mapping that used to live inside the (gpm-specific)
    // plugin and now lives here. A wrong alias/policy would silently mis-seal a
    // key under the wrong Keystore entry or a wrong key-generation policy
    // (auth-free vs biometric-gated, enrollment invalidation). Driven through a
    // recording mock so no Android Keystore is needed.

    use tauri_plugin_secure_keystore::KeyPolicy as SecureKeyPolicy;
    type MockRetrieve = (String, String, SecureKeyPolicy);
    type MockStore = (String, String, String, SecureKeyPolicy);

    /// Recording [`KvKeystore`] mock: captures every call's alias/prefs/policy and
    /// returns programmable values. `Mutex` (not `tokio::sync`) because no await
    /// happens while a lock is held.
    struct MockKeystore {
        retrieves: std::sync::Mutex<Vec<MockRetrieve>>,
        stores: std::sync::Mutex<Vec<MockStore>>,
        deletes: std::sync::Mutex<Vec<(String, String)>>,
        alias_states: std::sync::Mutex<Vec<(String, String)>>,
        retrieve_ret: std::sync::Mutex<Result<String, SecureKeystoreError>>,
        alias_state_ret: std::sync::Mutex<AliasState>,
        /// Per-alias overrides; when absent for a probed alias, falls back to
        /// `alias_state_ret`. Lets a test distinguish vault vs legacy probes.
        alias_state_overrides: std::sync::Mutex<std::collections::HashMap<String, AliasState>>,
    }

    impl MockKeystore {
        fn new() -> Self {
            Self {
                retrieves: std::sync::Mutex::new(Vec::new()),
                stores: std::sync::Mutex::new(Vec::new()),
                deletes: std::sync::Mutex::new(Vec::new()),
                alias_states: std::sync::Mutex::new(Vec::new()),
                retrieve_ret: std::sync::Mutex::new(Ok(String::new())),
                alias_state_ret: std::sync::Mutex::new(AliasState {
                    present: false,
                    usable: false,
                }),
                alias_state_overrides: std::sync::Mutex::new(std::collections::HashMap::new()),
            }
        }
        fn with_retrieve(self, r: Result<String, SecureKeystoreError>) -> Self {
            *self.retrieve_ret.lock().unwrap() = r;
            self
        }
        fn with_alias_state(self, s: AliasState) -> Self {
            *self.alias_state_ret.lock().unwrap() = s;
            self
        }
        /// Override the `alias_state` return for one alias only (e.g. vault vs
        /// legacy), so the m0007 pre/post-transition shapes are testable.
        fn with_alias_state_for(self, alias: &str, s: AliasState) -> Self {
            self.alias_state_overrides
                .lock()
                .unwrap()
                .insert(alias.to_string(), s);
            self
        }
    }

    impl KvKeystore for MockKeystore {
        async fn alias_state(
            &self,
            alias: &str,
            prefs: &str,
        ) -> Result<AliasState, SecureKeystoreError> {
            self.alias_states
                .lock()
                .unwrap()
                .push((alias.to_string(), prefs.to_string()));
            let s = self
                .alias_state_overrides
                .lock()
                .unwrap()
                .get(alias)
                .copied()
                .unwrap_or_else(|| *self.alias_state_ret.lock().unwrap());
            Ok(s)
        }
        async fn delete(&self, alias: &str, prefs: &str) -> Result<(), SecureKeystoreError> {
            self.deletes
                .lock()
                .unwrap()
                .push((alias.to_string(), prefs.to_string()));
            Ok(())
        }
        async fn store(
            &self,
            value: &str,
            alias: &str,
            prefs: &str,
            policy: tauri_plugin_secure_keystore::KeyPolicy,
            _prompt: Option<&tauri_plugin_secure_keystore::ResolvedPromptText>,
        ) -> Result<(), SecureKeystoreError> {
            self.stores.lock().unwrap().push((
                value.to_string(),
                alias.to_string(),
                prefs.to_string(),
                policy,
            ));
            Ok(())
        }
        async fn retrieve(
            &self,
            alias: &str,
            prefs: &str,
            policy: tauri_plugin_secure_keystore::KeyPolicy,
            _prompt: Option<&tauri_plugin_secure_keystore::ResolvedPromptText>,
        ) -> Result<String, SecureKeystoreError> {
            self.retrieves
                .lock()
                .unwrap()
                .push((alias.to_string(), prefs.to_string(), policy));
            self.retrieve_ret.lock().unwrap().clone()
        }
    }

    #[tokio::test]
    async fn retrieve_master_passes_master_alias_and_free_policy() {
        let mock = MockKeystore::new().with_retrieve(Ok("key".to_string()));
        assert_eq!(
            retrieve_master(&mock).await.unwrap().as_deref(),
            Some("key")
        );
        assert_eq!(
            *mock.retrieves.lock().unwrap(),
            vec![(
                MASTER_ALIAS.to_string(),
                MASTER_PREFS.to_string(),
                MASTER_FREE_POLICY
            )]
        );
    }

    #[tokio::test]
    async fn retrieve_master_maps_not_set_to_none() {
        let mock = MockKeystore::new().with_retrieve(Err(SecureKeystoreError {
            code: "BIOMETRIC_NOT_SET".to_string(),
            message: String::new(),
        }));
        assert!(retrieve_master(&mock).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn store_master_passes_master_alias_and_free_policy() {
        let mock = MockKeystore::new();
        store_master(&mock, "key").await.unwrap();
        assert_eq!(
            *mock.stores.lock().unwrap(),
            vec![(
                "key".to_string(),
                MASTER_ALIAS.to_string(),
                MASTER_PREFS.to_string(),
                MASTER_FREE_POLICY
            )]
        );
    }

    #[tokio::test]
    async fn retrieve_slot_passes_each_slots_alias_and_vault_policy() {
        for slot in [BiometricSlot::Vault, BiometricSlot::Legacy] {
            let mock = MockKeystore::new().with_retrieve(Ok("k".to_string()));
            retrieve_slot(&mock, slot, None).await.unwrap();
            assert_eq!(
                *mock.retrieves.lock().unwrap(),
                vec![(
                    slot.alias().to_string(),
                    slot.prefs().to_string(),
                    VAULT_POLICY
                )],
                "{slot:?} alias/prefs/policy"
            );
        }
    }

    #[tokio::test]
    async fn store_slot_passes_each_slots_alias_and_vault_policy() {
        for slot in [BiometricSlot::Vault, BiometricSlot::Legacy] {
            let mock = MockKeystore::new();
            store_slot(&mock, "v", slot, None).await.unwrap();
            assert_eq!(
                *mock.stores.lock().unwrap(),
                vec![(
                    "v".to_string(),
                    slot.alias().to_string(),
                    slot.prefs().to_string(),
                    VAULT_POLICY
                )],
                "{slot:?} alias/prefs/policy"
            );
        }
    }

    #[tokio::test]
    async fn delete_slot_passes_slot_alias_and_prefs() {
        let mock = MockKeystore::new();
        delete_slot(&mock, BiometricSlot::Vault).await.unwrap();
        delete_slot(&mock, BiometricSlot::Legacy).await.unwrap();
        let d = mock.deletes.lock().unwrap();
        assert_eq!(
            *d,
            vec![
                (
                    BiometricSlot::Vault.alias().to_string(),
                    BiometricSlot::Vault.prefs().to_string()
                ),
                (
                    BiometricSlot::Legacy.alias().to_string(),
                    BiometricSlot::Legacy.prefs().to_string()
                ),
            ]
        );
    }

    #[tokio::test]
    async fn has_app_lock_enabled_true_short_circuits_on_vault_probe() {
        // A usable vault key ⇒ true, and the legacy slot is NOT probed.
        let mock = MockKeystore::new().with_alias_state(AliasState {
            present: true,
            usable: true,
        });
        assert!(has_app_lock_enabled(&mock).await);
        let probes = mock.alias_states.lock().unwrap();
        assert_eq!(
            *probes,
            vec![(
                BiometricSlot::Vault.alias().to_string(),
                BiometricSlot::Vault.prefs().to_string()
            )]
        );
    }

    #[tokio::test]
    async fn has_app_lock_enabled_false_probes_both_slots() {
        // Neither slot usable ⇒ false, and BOTH slots are probed (vault then
        // legacy) — pinning the dual-alias m0007-transition coverage.
        let mock = MockKeystore::new();
        assert!(!has_app_lock_enabled(&mock).await);
        let probes = mock.alias_states.lock().unwrap();
        assert_eq!(
            *probes,
            vec![
                (
                    BiometricSlot::Vault.alias().to_string(),
                    BiometricSlot::Vault.prefs().to_string()
                ),
                (
                    BiometricSlot::Legacy.alias().to_string(),
                    BiometricSlot::Legacy.prefs().to_string()
                ),
            ]
        );
    }

    #[tokio::test]
    async fn has_app_lock_enabled_false_when_present_but_unusable() {
        // A present-but-DEAD key (present:true, usable:false — the
        // all-biometrics-removed re-setup case) must NOT arm the gate. Guards
        // against `present && usable` regressing to just `present`.
        let mock = MockKeystore::new().with_alias_state(AliasState {
            present: true,
            usable: false,
        });
        assert!(!has_app_lock_enabled(&mock).await);
        // Both slots probed (vault dead ⇒ fall through to legacy).
        assert_eq!(mock.alias_states.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn has_app_lock_enabled_true_when_only_legacy_usable() {
        // Pre-m0007 upgrader: vault absent, legacy holds the usable key ⇒ true.
        // Requires per-alias returns (vault ≠ legacy).
        let mock = MockKeystore::new()
            .with_alias_state_for(
                BiometricSlot::Vault.alias(),
                AliasState {
                    present: false,
                    usable: false,
                },
            )
            .with_alias_state_for(
                BiometricSlot::Legacy.alias(),
                AliasState {
                    present: true,
                    usable: true,
                },
            );
        assert!(has_app_lock_enabled(&mock).await);
        // Vault probed first (absent), then legacy (usable) — both fire in order.
        assert_eq!(
            *mock.alias_states.lock().unwrap(),
            vec![
                (
                    BiometricSlot::Vault.alias().to_string(),
                    BiometricSlot::Vault.prefs().to_string()
                ),
                (
                    BiometricSlot::Legacy.alias().to_string(),
                    BiometricSlot::Legacy.prefs().to_string()
                ),
            ]
        );
    }

    #[tokio::test]
    async fn retrieve_slot_maps_not_set_to_none() {
        // A missing slot returns None with no prompt — the docstring promise.
        for slot in [BiometricSlot::Vault, BiometricSlot::Legacy] {
            let mock = MockKeystore::new().with_retrieve(Err(SecureKeystoreError {
                code: "BIOMETRIC_NOT_SET".to_string(),
                message: String::new(),
            }));
            assert!(
                retrieve_slot(&mock, slot, None).await.unwrap().is_none(),
                "{slot:?}: NOT_SET should map to None"
            );
        }
    }

    #[tokio::test]
    async fn retrieve_master_maps_unavailable_to_none() {
        // The desktop / no-Keystore branch (fires on every desktop launch).
        let mock = MockKeystore::new().with_retrieve(Err(SecureKeystoreError {
            code: "SECURE_KEYSTORE_UNAVAILABLE".to_string(),
            message: String::new(),
        }));
        assert!(retrieve_master(&mock).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn retrieve_propagates_non_not_set_errors() {
        // A real failure (key invalidated / cancelled / failed) must NOT collapse
        // to Ok(None) — it must propagate so the caller can react.
        for code in [
            "BIOMETRIC_KEY_INVALIDATED",
            "BIOMETRIC_CANCELLED",
            "BIOMETRIC_FAILED",
        ] {
            let mock = MockKeystore::new().with_retrieve(Err(SecureKeystoreError {
                code: code.to_string(),
                message: String::new(),
            }));
            let err = retrieve_master(&mock).await.unwrap_err();
            assert_eq!(err.code, code, "{code}: should propagate, not map to None");
            let mock2 = MockKeystore::new().with_retrieve(Err(SecureKeystoreError {
                code: code.to_string(),
                message: String::new(),
            }));
            let err2 = retrieve_slot(&mock2, BiometricSlot::Vault, None)
                .await
                .unwrap_err();
            assert_eq!(err2.code, code, "{code}: retrieve_slot should propagate");
        }
    }
}
