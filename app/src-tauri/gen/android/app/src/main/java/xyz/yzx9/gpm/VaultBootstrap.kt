// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package xyz.yzx9.gpm

import android.content.Context
import android.content.SharedPreferences
import android.util.Base64
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.spec.GCMParameterSpec

/**
 * Retrieves the **biometric-gated** vault key (the one sealing `identity`;
 * `gpm_vault_key`) directly from the Android Keystore — the vault-key
 * counterpart of [HeadlessBootstrap]'s auth-free retrieve, for OS-started
 * entry points with no Tauri `AppHandle`.
 *
 * `KEY_ALIAS`/`PREFS_NAME` are the Kotlin mirrors of the Rust
 * `BiometricSlot::Vault` consts in `app/src-tauri/src/keystore.rs` — keep
 * them in sync on rename (the duplication is inherent: Kotlin can't read
 * Rust consts). The cipher/prompt sequence mirrors the keystore plugin's
 * private retrieve (the ~30-line mirror is the R077-deferred dedup, noted
 * for the autofill follow-up RFC): sealed iv/ct are read BEFORE any prompt;
 * `Cipher.init` on the auth-bound key never prompts — only `doFinal` does —
 * so the STRONG prompt is the last, user-visible step.
 */
object VaultBootstrap {
    private const val ANDROID_KEYSTORE = "AndroidKeyStore"
    internal const val KEY_ALIAS = "gpm_vault_key"
    internal const val PREFS_NAME = "gpm_secure_keystoreVault"
    internal const val PREF_IV = "iv"
    internal const val PREF_CT = "ct"
    private const val GCM_TAG_BITS = 128

    /** Non-prompting: is a vault key sealed (≈ App Lock on)? Prefs first. */
    fun isVaultSealed(context: Context): Boolean {
        val prefs = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
        return !prefs.getString(PREF_IV, null).isNullOrEmpty() &&
            !prefs.getString(PREF_CT, null).isNullOrEmpty()
    }

    /**
     * Testable seam (the HeadlessBootstrap pattern): the sealed blob against
     * an explicit [KeyStore] + [SharedPreferences] pair, or `null` when the
     * alias or either prefs value is missing — the null branches run under
     * Robolectric with an ordinary keystore; the prompt/doFinal path needs a
     * real hardware-backed key and is covered by the device smoke.
     */
    internal fun sealedBlob(
        keyStore: KeyStore,
        prefs: SharedPreferences,
    ): Pair<ByteArray, ByteArray>? {
        if (!keyStore.containsAlias(KEY_ALIAS)) return null
        val ivB64 = prefs.getString(PREF_IV, null) ?: return null
        val ctB64 = prefs.getString(PREF_CT, null) ?: return null
        return Pair(Base64.decode(ivB64, Base64.NO_WRAP), Base64.decode(ctB64, Base64.NO_WRAP))
    }

    /**
     * STRONG-biometric DECRYPT of the vault key. Hands the raw 32 bytes,
     * base64 (STANDARD, NO_WRAP — the Rust↔Keystore IPC shape), to
     * [onUnsealed]; any prompt error or pre-prompt failure goes to [onError].
     * The host must be a [FragmentActivity] (the BiometricPrompt contract).
     */
    fun unsealVaultKey(
        activity: FragmentActivity,
        onUnsealed: (String) -> Unit,
        onError: (code: Int, message: CharSequence) -> Unit,
    ) {
        val keyStore = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        val prefs = activity.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
        val (iv, ct) = sealedBlob(keyStore, prefs) ?: run {
            onError(0, "vault key not set")
            return
        }
        // Key access + Cipher.init run pre-prompt and can throw (a key
        // invalidated by biometric re-enrollment, an OEM keystore failure) —
        // fail closed to onError, never crash the fill surface.
        val cipher =
            try {
                val key =
                    (keyStore.getEntry(KEY_ALIAS, null) as? KeyStore.SecretKeyEntry)?.secretKey
                        ?: run {
                            onError(0, "vault key unavailable")
                            return
                        }
                Cipher.getInstance("AES/GCM/NoPadding").apply {
                    init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(GCM_TAG_BITS, iv))
                }
            } catch (e: Exception) {
                onError(0, "vault key unavailable")
                return
            }

        val prompt =
            BiometricPrompt(
                activity,
                ContextCompat.getMainExecutor(activity),
                object : BiometricPrompt.AuthenticationCallback() {
                    override fun onAuthenticationError(code: Int, errString: CharSequence) {
                        onError(code, errString)
                    }

                    override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                        val authCipher =
                            result.cryptoObject?.cipher ?: run {
                                onError(0, "cipher missing after auth")
                                return
                            }
                        // A tag failure (stale prefs vs a re-sealed key) must
                        // fail closed, not crash the prompt callback.
                        val plain =
                            try {
                                authCipher.doFinal(ct)
                            } catch (e: Exception) {
                                onError(0, "vault key decrypt failed")
                                return
                            }
                        onUnsealed(Base64.encodeToString(plain, Base64.NO_WRAP))
                    }
                },
            )
        prompt.authenticate(promptInfo(), BiometricPrompt.CryptoObject(cipher))
    }

    /** STRONG-only; the title/negative strings are the Rust fallback pair. */
    internal fun promptInfo(): BiometricPrompt.PromptInfo =
        BiometricPrompt.PromptInfo.Builder()
            .setTitle("gpm")
            .setNegativeButtonText("Cancel")
            .setAllowedAuthenticators(BiometricManager.Authenticators.BIOMETRIC_STRONG)
            .build()

    /** Pure mapper over `BiometricManager.canAuthenticate(BIOMETRIC_STRONG)`. */
    fun strongBiometricAvailable(canAuthenticate: Int): Boolean =
        canAuthenticate == BiometricManager.BIOMETRIC_SUCCESS
}
