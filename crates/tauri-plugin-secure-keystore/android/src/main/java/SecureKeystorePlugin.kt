// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0
//
// Generic Android Keystore storage for a caller-supplied secret string, under a
// caller-chosen policy: auth-free (no prompt, survives biometric changes) OR
// biometric-gated (a BiometricPrompt per use). The alias, prefs name,
// key-generation policy, and prompt text are ALL caller-supplied — this plugin
// carries no app-specific identifiers or brand strings.
//
// The crypto pattern (AndroidKeyStore AES/GCM key, optionally bound to a
// BiometricPrompt CryptoObject with prompts on BOTH encrypt and decrypt) mirrors
// KeystorePlugin.kt in biometric-keystore; the two are homomorphic so they can
// be merged mechanically later.
//
// Secrets are handled as ByteArray within the crypto flow and zeroed after use.

package xyz.yzx9.gpm.securekeystore

import android.app.Activity
import android.content.Context
import android.content.SharedPreferences
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import android.util.Log
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
private const val PREF_CT = "ct"
private const val PREF_IV = "iv"

private val UTF_8: Charset = Charsets.UTF_8

/** GCM authentication tag length, in bits. */
private const val GCM_TAG_BITS = 128

/** Map a [BiometricPrompt] error code to a stable `BIOMETRIC_*` code. Pure: the
 *  `ERROR_*` constants are compile-time-inlined `static final int`, so the
 *  extracted function carries no runtime dependency on `androidx.biometric`. */
internal fun mapErrorCode(code: Int): String = when (code) {
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
internal fun safeName(e: Throwable): String = e.javaClass.simpleName.ifEmpty { "error" }

/** Map the STRONG + WEAK `BiometricManager.canAuthenticate` results to a stable
 *  availability-state string (consumed by Rust as `BiometricState`). Pure: the
 *  `BIOMETRIC_*` constants are compile-time-inlined `static final int`, so this
 *  is a plain JVM unit test — exhaustive over every return.
 *  Duplicated from KeystorePlugin.kt (biometric-keystore) because the plugins are
 *  separate Gradle modules; the cross-layer string contract is pinned by tests.
 *
 *  - `SUCCESS` → "available"
 *  - `NONE_ENROLLED` + a weak print enrolled → "weak_enrolled"
 *  - `NONE_ENROLLED` + nothing enrolled → "no_enrollment"
 *  - anything else → "unavailable"; pre-API-30 folds to "unavailable" in
 *    [isBiometricAvailable] before this runs. */
internal fun mapBiometricState(strongCode: Int, weakCode: Int): String = when {
    strongCode == BiometricManager.BIOMETRIC_SUCCESS -> "available"
    strongCode == BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED &&
        weakCode == BiometricManager.BIOMETRIC_SUCCESS -> "weak_enrolled"
    strongCode == BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED -> "no_enrollment"
    else -> "unavailable"
}

/** Base64-encode the iv + ciphertext for SharedPreferences storage. Pure. */
internal fun encodeBlob(iv: ByteArray, ciphertext: ByteArray): Pair<String, String> =
    Pair(Base64.encodeToString(iv, Base64.NO_WRAP), Base64.encodeToString(ciphertext, Base64.NO_WRAP))

/** Decode the stored base64 iv + ciphertext. Returns null iff either input is
 *  null (i.e. nothing is sealed); a present-but-empty string decodes to an empty
 *  `ByteArray` — preserving the original `readCipherData` semantics exactly
 *  (characterization, not a behavior change). */
internal fun decodeBlob(ivB64: String?, ctB64: String?): Pair<ByteArray, ByteArray>? {
    if (ivB64 == null || ctB64 == null) return null
    return Pair(Base64.decode(ivB64, Base64.NO_WRAP), Base64.decode(ctB64, Base64.NO_WRAP))
}

/** Caller-supplied key-generation policy (mirrors the Rust `KeyPolicy`). The
 *  plugin applies it verbatim to [KeyGenParameterSpec]; it never invents values. */
@InvokeArg
class KeyPolicyArgs {
    var authRequired: Boolean = false
    var authBiometricStrong: Boolean = false
    var invalidatedByEnrollment: Boolean = false
    var authValiditySeconds: Long = 0
}

/** `store` args: the secret + alias/prefs + policy + prompt text. */
@InvokeArg
class StoreArgs {
    lateinit var value: String
    lateinit var alias: String
    lateinit var prefs: String
    var policy: KeyPolicyArgs? = null
    var title: String? = null
    var subtitle: String? = null
    var negative: String? = null
}

/** `retrieve` args: alias/prefs + policy + prompt text (carries no secret). */
@InvokeArg
class RetrieveArgs {
    lateinit var alias: String
    lateinit var prefs: String
    var policy: KeyPolicyArgs? = null
    var title: String? = null
    var subtitle: String? = null
    var negative: String? = null
}

/** `aliasState` / `delete` args: which alias + prefs to probe/clear. */
@InvokeArg
class AliasArgs {
    lateinit var alias: String
    lateinit var prefs: String
}

/**
 * Generic Android Keystore storage. All identifiers (alias, prefs) and policy
 * come from the caller; this class carries no app-specific state.
 *
 * The auth-free path is API 23+ (minSdk 24). The biometric-gated path is API
 * 30+ (Android 11): its key uses
 * [KeyGenParameterSpec.Builder.setUserAuthenticationParameters], so every
 * encrypt/decrypt requires a CryptoObject-bound STRONG biometric prompt.
 */
@TauriPlugin
class SecureKeystorePlugin(private val activity: Activity) : Plugin(activity) {

    // ── Lifecycle-free helpers ───────────────────────────────────────────

    /** The host activity as a [FragmentActivity], required by [BiometricPrompt]. */
    private fun fragmentActivity(): FragmentActivity? = activity as? FragmentActivity

    private fun prefs(name: String): SharedPreferences =
        activity.getSharedPreferences(name, Context.MODE_PRIVATE)

    /** The STRONG biometric authenticators bitmask. */
    private val strongAuthenticator: Int
        get() = BiometricManager.Authenticators.BIOMETRIC_STRONG

    // ── Key + cipher management ──────────────────────────────────────────

    /**
     * Generate a fresh key at [alias] per [policy], replacing any prior entry.
     *
     * A fresh key on every `store` sidesteps the "alias exists but key is
     * invalidated" trap. **Conditional policy application**: auth params and
     * `setInvalidatedByBiometricEnrollment` are applied ONLY when
     * `policy.authRequired` — an auth-free keygen calls neither, so its spec is
     * byte-identical to a plain keygen (no enrollment flag is set, which keeps
     * the "no migration" invariant honest: `unset` stays `unset`).
     */
    @RequiresApi(Build.VERSION_CODES.R)
    private fun generateKey(alias: String, policy: KeyPolicyArgs) {
        val keyStore = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        if (keyStore.containsAlias(alias)) {
            keyStore.deleteEntry(alias)
        }
        val keyGenerator =
            KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
        val builder = KeyGenParameterSpec.Builder(
            alias,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
        )
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
        if (policy.authRequired) {
            builder.setUserAuthenticationRequired(true)
            val authType = if (policy.authBiometricStrong) KeyProperties.AUTH_BIOMETRIC_STRONG else 0
            // API 30+: validity (0 = per-use) + authenticator set.
            builder.setUserAuthenticationParameters(
                policy.authValiditySeconds.toInt().coerceAtLeast(0),
                authType,
            )
            // Meaningful only for a user-auth-bound key; an auth-free keygen
            // never calls this, so the spec stays a plain keygen.
            builder.setInvalidatedByBiometricEnrollment(policy.invalidatedByEnrollment)
        }
        keyGenerator.init(builder.build())
        keyGenerator.generateKey()
    }

    private fun loadKey(alias: String): SecretKey {
        val keyStore = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        return (keyStore.getEntry(alias, null) as KeyStore.SecretKeyEntry).secretKey
    }

    /** A [Cipher] initialised for encryption with a fresh IV. */
    @RequiresApi(Build.VERSION_CODES.R)
    private fun encryptionCipher(alias: String): Cipher {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, loadKey(alias))
        return cipher
    }

    /** A [Cipher] initialised for decryption with the stored IV. */
    private fun decryptionCipher(alias: String, iv: ByteArray): Cipher {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, loadKey(alias), GCMParameterSpec(GCM_TAG_BITS, iv))
        return cipher
    }

    private fun storeCipherData(prefs: SharedPreferences, iv: ByteArray, ciphertext: ByteArray) {
        val (ivB64, ctB64) = encodeBlob(iv, ciphertext)
        prefs.edit().apply {
            putString(PREF_IV, ivB64)
            putString(PREF_CT, ctB64)
        }.apply()
    }

    /** The stored (iv, ciphertext) pair for [prefs], or null if nothing is sealed. */
    private fun readCipherData(prefs: SharedPreferences): Pair<ByteArray, ByteArray>? =
        decodeBlob(prefs.getString(PREF_IV, null), prefs.getString(PREF_CT, null))

    // ── PromptInfo (caller-supplied text; no brand fallback) ─────────────

    /** Build [BiometricPrompt.PromptInfo] from caller-supplied text. The plugin
     *  bakes NO fallback string — `title`/`negative` must be supplied by the
     *  caller when the policy is auth-required (a missing title rejects rather
     *  than silently substituting a brand). */
    private fun promptInfo(title: String?, subtitle: String?, negative: String?): BiometricPrompt.PromptInfo {
        val builder = BiometricPrompt.PromptInfo.Builder()
            .setTitle(title ?: error("auth-required policy needs a prompt title"))
            .setNegativeButtonText(negative ?: error("auth-required policy needs a negative label"))
            .setAllowedAuthenticators(strongAuthenticator)
        if (subtitle != null) builder.setSubtitle(subtitle)
        return builder.build()
    }

    // ── @Command surface ─────────────────────────────────────────────────

    /** Availability state for biometric-gated storage — one of "available",
     *  "no_enrollment", "weak_enrolled", "unavailable". Non-prompting.
     *  Pre-API-30 → "unavailable" (the STRONG keystore key requires R). */
    @Command
    fun isBiometricAvailable(invoke: Invoke) {
        val state = if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) {
            "unavailable"
        } else {
            val bm = BiometricManager.from(activity)
            mapBiometricState(
                bm.canAuthenticate(strongAuthenticator),
                bm.canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_WEAK),
            )
        }
        val ret = JSObject()
        ret.put("state", state)
        invoke.resolve(ret)
    }

    /**
     * Probe one alias's liveness: `{ present, usable }`. Non-prompting.
     * `present` = ciphertext exists; `usable` = the key still initializes
     * (pre-API-30 or a dead key → false). This is the primitive the Rust side
     * composes (`present && usable`).
     */
    @Command
    fun aliasState(invoke: Invoke) {
        val args = invoke.parseArgs(AliasArgs::class.java)
        val p = prefs(args.prefs)
        val present = readCipherData(p) != null
        // The R-only cipher probe must never be touched on API <30.
        val usable = present &&
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.R &&
            cipherUsable(args.alias)
        val ret = JSObject()
        ret.put("present", present)
        ret.put("usable", usable)
        invoke.resolve(ret)
    }

    /** Whether [alias]'s sealed key still inits — i.e. is usable. Non-prompting:
     *  init on an authentication-bound key does NOT require auth; only the
     *  prompt does. Any init failure ⇒ not usable ⇒ fall back / re-setup. */
    @RequiresApi(Build.VERSION_CODES.R)
    private fun cipherUsable(alias: String): Boolean = try {
        encryptionCipher(alias)
        true
    } catch (e: Exception) {
        Log.w("gpm_secure_keystore", "aliasState probe failed ($alias): ${safeName(e)}")
        false
    }

    /**
     * Seal the supplied value at `alias` behind biometric auth (when the policy
     * is auth-required). Shows a CryptoObject ENCRYPT prompt and resolves ONLY
     * from a terminal biometric callback. Auth-free policy seals directly.
     */
    @RequiresApi(Build.VERSION_CODES.R)
    @Command
    fun store(invoke: Invoke) {
        val args = invoke.parseArgs(StoreArgs::class.java)
        val policy = args.policy ?: KeyPolicyArgs()
        val plainBytes = args.value.toByteArray(UTF_8)

        val cipher = try {
            generateKey(args.alias, policy)
            encryptionCipher(args.alias)
        } catch (e: Exception) {
            plainBytes.fill(0)
            invoke.reject(safeName(e), "BIOMETRIC_FAILED")
            return
        }

        if (!policy.authRequired) {
            // Auth-free keygen: seal directly, no prompt.
            try {
                val ciphertext = cipher.doFinal(plainBytes)
                storeCipherData(prefs(args.prefs), cipher.iv, ciphertext)
                ciphertext.fill(0)
                invoke.resolve()
            } catch (e: Exception) {
                invoke.reject(safeName(e), "BIOMETRIC_FAILED")
            } finally {
                plainBytes.fill(0)
            }
            return
        }

        val fa = fragmentActivity() ?: run {
            plainBytes.fill(0)
            invoke.reject("not FragmentActivity", "BIOMETRIC_UNAVAILABLE")
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
                        storeCipherData(prefs(args.prefs), authCipher.iv, ciphertext)
                        ciphertext.fill(0)
                        invoke.resolve()
                    } catch (e: Exception) {
                        invoke.reject(safeName(e), "BIOMETRIC_FAILED")
                    } finally {
                        plainBytes.fill(0)
                    }
                }

                override fun onAuthenticationError(code: Int, errString: CharSequence) {
                    plainBytes.fill(0)
                    invoke.reject(errString.toString(), mapErrorCode(code))
                }

                // Non-terminal (wrong finger): leave the prompt open.
                override fun onAuthenticationFailed() {}
            },
        )

        prompt.authenticate(
            promptInfo(args.title, args.subtitle, args.negative),
            BiometricPrompt.CryptoObject(cipher),
        )
    }

    /**
     * Retrieve the sealed value at `alias`. Auth-free policy decrypts directly
     * (no prompt); auth-required policy shows a CryptoObject DECRYPT prompt and
     * resolves ONLY from a terminal callback. Rejects with `BIOMETRIC_NOT_SET`
     * when nothing is sealed (before any prompt).
     */
    @Command
    fun retrieve(invoke: Invoke) {
        val args = invoke.parseArgs(RetrieveArgs::class.java)
        val policy = args.policy ?: KeyPolicyArgs()

        val (iv, ciphertext) = readCipherData(prefs(args.prefs)) ?: run {
            invoke.reject("nothing stored", "BIOMETRIC_NOT_SET")
            return
        }

        if (!policy.authRequired) {
            // Auth-free keygen: decrypt directly, no prompt.
            try {
                val cipher = decryptionCipher(args.alias, iv)
                val plain = cipher.doFinal(ciphertext)
                val ret = JSObject()
                ret.put("value", String(plain, UTF_8))
                invoke.resolve(ret)
                plain.fill(0)
            } catch (e: Exception) {
                invoke.reject(safeName(e), "BIOMETRIC_FAILED")
            }
            return
        }

        val fa = fragmentActivity() ?: run {
            invoke.reject("not FragmentActivity", "BIOMETRIC_UNAVAILABLE")
            return
        }
        val cipher = try {
            decryptionCipher(args.alias, iv)
        } catch (e: Exception) {
            // Includes KeyPermanentlyInvalidatedException when all biometrics
            // were removed since the key was generated → re-setup required.
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
                        ret.put("value", String(plain, UTF_8))
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

        prompt.authenticate(
            promptInfo(args.title, args.subtitle, args.negative),
            BiometricPrompt.CryptoObject(cipher),
        )
    }

    /** Delete the Keystore key at `alias` and the stored ciphertext (best-effort). */
    @Command
    fun delete(invoke: Invoke) {
        val args = invoke.parseArgs(AliasArgs::class.java)
        try {
            val keyStore = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
            if (keyStore.containsAlias(args.alias)) {
                keyStore.deleteEntry(args.alias)
            }
        } catch (_: Exception) {
            // Best-effort: still clear prefs so the app can always reset.
        }
        prefs(args.prefs).edit().clear().apply()
        invoke.resolve()
    }
}
