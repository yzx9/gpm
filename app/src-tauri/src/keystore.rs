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

use tauri_plugin_biometric_keystore::{
    KeyPolicy, PromptText, ResolvedPromptText, resolve_prompt_text,
};

// ---------------------------------------------------------------------------
// Identity passphrase slot (biometric-gated, biometric-keystore plugin)
// ---------------------------------------------------------------------------

/// Keystore alias for the sealed identity passphrase.
pub(crate) const PASSPHRASE_ALIAS: &str = "gpm_passphrase";
/// SharedPreferences file holding the sealed passphrase ciphertext.
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
