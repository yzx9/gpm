// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0
//
// Biometric-gated Android Keystore storage for the gpm identity passphrase.
//
// The crypto pattern (AndroidKeyStore AES/GCM key bound to a BiometricPrompt
// CryptoObject, with prompts on BOTH encrypt and decrypt) is adapted from
// impierce/tauri-plugin-keystore (Apache-2.0), which is the reviewed
// reference called out in the design plan. The Tauri @Command/@Invoke shape
// mirrors SafeAreaPlugin.kt.
//
// Secrets are handled as ByteArray within the crypto flow and zeroed after
// use. (A JVM String is unavoidable at the invoke.resolve hop that crosses
// back to Rust — see the plan's Finding 2 — and lives only briefly.)

package xyz.yzx9.gpm.biometrickeystore

import android.app.Activity
import android.content.Context
import android.content.SharedPreferences
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import androidx.annotation.RequiresApi
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.nio.charset.Charset
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

private const val ANDROID_KEYSTORE = "AndroidKeyStore"
private const val KEY_ALIAS = "gpm_passphrase"
private const val PREFS_NAME = "gpm_keystore"
private const val PREF_CT = "ct"
private const val PREF_IV = "iv"

private val UTF_8: Charset = Charsets.UTF_8

/** GCM authentication tag length, in bits. */
private const val GCM_TAG_BITS = 128

@InvokeArg
class StoreArgs {
    lateinit var passphrase: String
}

/**
 * Stores the gpm identity passphrase in the Android Keystore, sealed with a
 * hardware-backed AES/GCM key that is gated behind STRONG biometric auth.
 *
 * Biometric is API 30+ (Android 11+) only: the key uses
 * [KeyGenParameterSpec.Builder.setUserAuthenticationParameters] (API 30), so
 * every encrypt/decrypt requires a CryptoObject-bound biometric prompt.
 */
@TauriPlugin
class KeystorePlugin(private val activity: Activity) : Plugin(activity) {

    // ── Lifecycle-free helpers ───────────────────────────────────────────

    /** The host activity as a [FragmentActivity], required by [BiometricPrompt]. */
    private fun fragmentActivity(): FragmentActivity? =
        activity as? FragmentActivity

    private fun prefs(): SharedPreferences =
        activity.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    /** The STRONG biometric authenticators bitmask — must match the key's
     *  `AUTH_BIOMETRIC_STRONG` or `retrieve` fails at prompt time (plan A3). */
    private val strongAuthenticator: Int
        get() = BiometricManager.Authenticators.BIOMETRIC_STRONG

    // ── Key + cipher management ──────────────────────────────────────────

    /**
     * Generate a fresh biometric-gated AES/GCM key, replacing any prior entry.
     *
     * A fresh key on every `store` sidesteps the "alias exists but key is
     * invalidated" trap (a dead key keeps its alias), so re-enabling after a
     * fingerprint change just works.
     *
     * API 30+: `setUserAuthenticationParameters(0, AUTH_BIOMETRIC_STRONG)`
     * replaces the deprecated validity-duration call and forces per-use auth.
     */
    @RequiresApi(Build.VERSION_CODES.R)
    private fun generateKey() {
        val keyStore = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        if (keyStore.containsAlias(KEY_ALIAS)) {
            keyStore.deleteEntry(KEY_ALIAS)
        }
        val keyGenerator =
            KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
        val spec = KeyGenParameterSpec.Builder(
            KEY_ALIAS,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
        )
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setUserAuthenticationRequired(true)
            // API 30+: every use requires a CryptoObject-bound STRONG biometric.
            .setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG)
            .setInvalidatedByBiometricEnrollment(true)
            .build()
        keyGenerator.init(spec)
        keyGenerator.generateKey()
    }

    private fun loadKey(): SecretKey {
        val keyStore = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        return (keyStore.getEntry(KEY_ALIAS, null) as KeyStore.SecretKeyEntry).secretKey
    }

    /** A [Cipher] initialised for encryption with a fresh IV. */
    @RequiresApi(Build.VERSION_CODES.R)
    private fun encryptionCipher(): Cipher {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, loadKey())
        return cipher
    }

    /** A [Cipher] initialised for decryption with the stored IV. */
    private fun decryptionCipher(iv: ByteArray): Cipher {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, loadKey(), GCMParameterSpec(GCM_TAG_BITS, iv))
        return cipher
    }

    private fun storeCipherData(iv: ByteArray, ciphertext: ByteArray) {
        prefs().edit().apply {
            putString(PREF_IV, Base64.encodeToString(iv, Base64.NO_WRAP))
            putString(PREF_CT, Base64.encodeToString(ciphertext, Base64.NO_WRAP))
        }.apply()
    }

    /** The stored (iv, ciphertext) pair, or null if nothing is sealed. */
    private fun readCipherData(): Pair<ByteArray, ByteArray>? {
        val prefs = prefs()
        val ivB64 = prefs.getString(PREF_IV, null) ?: return null
        val ctB64 = prefs.getString(PREF_CT, null) ?: return null
        return Pair(Base64.decode(ivB64, Base64.NO_WRAP), Base64.decode(ctB64, Base64.NO_WRAP))
    }

    // ── PromptInfo ───────────────────────────────────────────────────────

    private fun promptInfo(title: String): BiometricPrompt.PromptInfo =
        BiometricPrompt.PromptInfo.Builder()
            .setTitle(title)
            .setSubtitle("Authenticate to access gpm")
            .setNegativeButtonText("Use passphrase")
            .setAllowedAuthenticators(strongAuthenticator)
            .build()

    /** Map a [BiometricPrompt] error code to a stable `BIOMETRIC_*` code. */
    private fun mapErrorCode(code: Int): String = when (code) {
        BiometricPrompt.ERROR_USER_CANCELED,
        BiometricPrompt.ERROR_NEGATIVE_BUTTON,
        BiometricPrompt.ERROR_CANCELED,
        -> "BIOMETRIC_CANCELLED"
        BiometricPrompt.ERROR_HW_NOT_PRESENT,
        BiometricPrompt.ERROR_HW_UNAVAILABLE,
        BiometricPrompt.ERROR_NO_BIOMETRICS,
        BiometricPrompt.ERROR_NO_DEVICE_CREDENTIAL,
        BiometricPrompt.ERROR_SECURITY_UPDATE_REQUIRED,
        -> "BIOMETRIC_UNAVAILABLE"
        BiometricPrompt.ERROR_LOCKOUT,
        BiometricPrompt.ERROR_LOCKOUT_PERMANENT,
        -> "BIOMETRIC_LOCKOUT"
        else -> "BIOMETRIC_FAILED"
    }

    /** Class name only — never leak crypto internals or secret data. */
    private fun safeName(e: Throwable): String = e.javaClass.simpleName.ifEmpty { "error" }

    // ── @Command surface ─────────────────────────────────────────────────

    /** `true` on API 30+ with a STRONG biometric enrolled. Non-prompting. */
    @Command
    fun is_available(invoke: Invoke) {
        val available = Build.VERSION.SDK_INT >= Build.VERSION_CODES.R &&
            BiometricManager.from(activity)
                .canAuthenticate(strongAuthenticator) == BiometricManager.BIOMETRIC_SUCCESS
        val ret = JSObject()
        ret.put("available", available)
        invoke.resolve(ret)
    }

    /** `true` if a sealed passphrase exists in prefs. Non-prompting. */
    @Command
    fun has_stored(invoke: Invoke) {
        val ret = JSObject()
        ret.put("stored", readCipherData() != null)
        invoke.resolve(ret)
    }

    /**
     * Seal the supplied passphrase behind biometric auth.
     *
     * Shows a CryptoObject ENCRYPT prompt (D2): a key with
     * `setUserAuthenticationRequired` needs auth for encrypt too. Resolves
     * ONLY from a terminal biometric callback — never synchronously — so the
     * `Invoke` stays open across the prompt.
     */
    @RequiresApi(Build.VERSION_CODES.R)
    @Command
    fun store(invoke: Invoke) {
        val fa = fragmentActivity() ?: run {
            invoke.reject("not FragmentActivity", "BIOMETRIC_UNAVAILABLE")
            return
        }

        val passphrase = invoke.parseArgs(StoreArgs::class.java).passphrase
        val plainBytes = passphrase.toByteArray(UTF_8)

        val cipher = try {
            generateKey()
            encryptionCipher()
        } catch (e: Exception) {
            plainBytes.fill(0)
            invoke.reject(safeName(e), "BIOMETRIC_FAILED")
            return
        }

        val prompt = BiometricPrompt(
            fa,
            ContextCompat.getMainExecutor(activity),
            object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    try {
                        val authCipher = result.cryptoObject?.cipher
                            ?: error("cipher missing after auth")
                        val ciphertext = authCipher.doFinal(plainBytes)
                        val iv = authCipher.iv
                        storeCipherData(iv, ciphertext)
                        ciphertext.fill(0)
                        invoke.resolve(JSObject())
                    } catch (e: Exception) {
                        invoke.reject(safeName(e), "BIOMETRIC_FAILED")
                    } finally {
                        plainBytes.fill(0)
                    }
                }

                override fun onAuthenticationError(code: Int, errString: CharSequence) {
                    invoke.reject(errString.toString(), mapErrorCode(code))
                }

                // Non-terminal (wrong finger): leave the prompt open. Do NOT
                // resolve/reject here — the user gets another attempt.
                override fun onAuthenticationFailed() {}
            },
        )

        prompt.authenticate(promptInfo("Enable biometric unlock"), BiometricPrompt.CryptoObject(cipher))
    }

    /**
     * Retrieve the sealed passphrase behind biometric auth.
     *
     * Shows a CryptoObject DECRYPT prompt and resolves ONLY from a terminal
     * callback. A permanently-invalidated key (new fingerprint enrolled) maps
     * to `BIOMETRIC_KEY_INVALIDATED` so the app can self-heal (delete + form).
     */
    @Command
    fun retrieve(invoke: Invoke) {
        val fa = fragmentActivity() ?: run {
            invoke.reject("not FragmentActivity", "BIOMETRIC_UNAVAILABLE")
            return
        }

        val (iv, ciphertext) = readCipherData() ?: run {
            invoke.reject("nothing stored", "BIOMETRIC_NOT_SET")
            return
        }

        val cipher = try {
            decryptionCipher(iv)
        } catch (e: Exception) {
            // Includes KeyPermanentlyInvalidatedException when a new biometric
            // was enrolled since the key was generated.
            invoke.reject(safeName(e), "BIOMETRIC_KEY_INVALIDATED")
            return
        }

        val prompt = BiometricPrompt(
            fa,
            ContextCompat.getMainExecutor(activity),
            object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    try {
                        val authCipher = result.cryptoObject?.cipher
                            ?: error("cipher missing after auth")
                        val plain = authCipher.doFinal(ciphertext)
                        val ret = JSObject()
                        ret.put("passphrase", String(plain, UTF_8))
                        invoke.resolve(ret)
                        plain.fill(0)
                    } catch (e: Exception) {
                        invoke.reject(safeName(e), "BIOMETRIC_FAILED")
                    }
                }

                override fun onAuthenticationError(code: Int, errString: CharSequence) {
                    invoke.reject(errString.toString(), mapErrorCode(code))
                }

                override fun onAuthenticationFailed() {}
            },
        )

        prompt.authenticate(promptInfo("Unlock gpm"), BiometricPrompt.CryptoObject(cipher))
    }

    /** Delete the Keystore key and the stored ciphertext (best-effort). */
    @Command
    fun delete(invoke: Invoke) {
        try {
            val keyStore = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
            if (keyStore.containsAlias(KEY_ALIAS)) {
                keyStore.deleteEntry(KEY_ALIAS)
            }
        } catch (e: Exception) {
            // Best-effort: still clear prefs and report success so the app can
            // always escape a stuck "enabled" state.
        }
        prefs().edit().clear().apply()
        invoke.resolve(JSObject())
    }
}
