// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0
//
// Android Keystore storage for the gpm at-rest **master key**, in two policies:
//
// 1. Auth-free (the default): sealed with a hardware-backed AES/GCM key that is
//    NOT user-authentication-required and NOT invalidated by biometric
//    enrollment — no BiometricPrompt, survives fingerprint changes. Used when
//    the app-launch biometric gate is OFF.
// 2. Biometric-gated (opt-in app-lock): the same master key blob, re-sealed
//    behind a key that IS user-authentication-required (STRONG biometric per
//    use) but still NOT invalidated by biometric enrollment. Adding a
//    fingerprint does NOT brick the store; only removing ALL biometrics does
//    (documented re-setup). The master key cannot self-heal the way a passphrase
//    can, so we deliberately diverge from KeystorePlugin (biometric-keystore),
//    which uses setInvalidatedByBiometricEnrollment(true) for the passphrase.
//
// The master key is a random 32-byte secret; the plugin seals it (iv +
// ciphertext in SharedPreferences) and hands the plaintext back to Rust (Base64
// over IPC). The non-extractable Keystore key never leaves the secure element.
//
// The biometric CryptoObject + BiometricPrompt pattern (prompt on BOTH encrypt
// and decrypt) mirrors KeystorePlugin.kt in biometric-keystore.

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
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

private const val ANDROID_KEYSTORE = "AndroidKeyStore"

// Auth-free master key.
private const val KEY_ALIAS = "gpm_master_key"
private const val PREFS_NAME = "gpm_secure_keystore"

// Biometric-gated master key (app-lock). Separate alias + prefs so the two
// policies never collide and a migration is just move-between-stores.
private const val BIOMETRIC_KEY_ALIAS = "gpm_master_key_biometric"
private const val BIOMETRIC_PREFS_NAME = "gpm_secure_keystoreBiometric"

private const val PREF_CT = "ct"
private const val PREF_IV = "iv"

/** GCM authentication tag length, in bits. */
private const val GCM_TAG_BITS = 128

// Vault key (R064): the biometric-gated key that seals `identity` + `app_id_pass`
// when App Lock is ON. Distinct alias + prefs from the legacy app-lock key so the
// at-rest master key (`gpm_master_key`) can stay auth-free + permanent while the
// identity moves under the gate. Same KeyGen spec as the legacy biometric key.
private const val VAULT_KEY_ALIAS = "gpm_vault_key"
private const val VAULT_PREFS_NAME = "gpm_secure_keystoreVault"

/**
 * Which biometric-gated key a biometric command targets. R064 splits the old
 * single `gpm_master_key_biometric` ([LEGACY] — held the master under App Lock
 * pre-R064) from the new `gpm_vault_key` ([VAULT] — holds the distinct vault
 * key). m0007 relocates LEGACY → the auth-free master + mints VAULT, then
 * deletes LEGACY.
 *
 * `internal` so the JVM unit test can exercise [fromString]; the alias/prefsName
 * are compile-time constants, so it is a plain JVM unit test.
 */
internal enum class BiometricSlot(val alias: String, val prefsName: String) {
    LEGACY(BIOMETRIC_KEY_ALIAS, BIOMETRIC_PREFS_NAME),
    VAULT(VAULT_KEY_ALIAS, VAULT_PREFS_NAME);

    companion object {
        /**
         * Parse the slot string sent by the Rust handle. Unspecified (`null`) ⇒
         * [LEGACY], preserving the pre-split bridge: until the app shell threads
         * an explicit slot, the biometric commands keep targeting the legacy
         * `gpm_master_key_biometric` alias exactly as before R064.
         */
        fun fromString(s: String?): BiometricSlot = when (s) {
            "vault" -> VAULT
            else -> LEGACY
        }
    }
}

/**
 * R064 dual-alias "app-lock is on" decision (pure, host-testable): `true` iff
 * EITHER biometric slot holds a sealed key that still inits cleanly. Pre-m0007
 * upgraders hold the legacy alias (vault absent); post-m0007 hold the vault
 * (legacy deleted); mutually exclusive in steady state. Extracted from
 * [SecureKeystorePlugin.hasStoredBiometric] so the OR-disjunction is pinned by
 * a plain JVM unit test (no Activity / Keystore needed).
 */
internal fun hasUsableBiometricInAnySlot(
    legacyPresent: Boolean,
    legacyUsable: Boolean,
    vaultPresent: Boolean,
    vaultUsable: Boolean,
): Boolean = (legacyPresent && legacyUsable) || (vaultPresent && vaultUsable)

/** Generic BiometricPrompt fallbacks (NOT a duplicate of
 *  native.json/en): the app name (title) + a neutral word (negative) keep the
 *  prompt coherent if the frontend omits the localized text. Duplicated from
 *  KeystorePlugin.kt because the plugins are separate Gradle modules. */
data class ResolvedPromptText(val title: String, val subtitle: String?, val negative: String)

/** Resolve localized prompt text against the generic fallbacks. Pure (no Android
 *  types) — plain JVM unit test. */
fun resolvePromptText(title: String?, subtitle: String?, negative: String?): ResolvedPromptText =
    ResolvedPromptText(
        title = title?.takeUnless { it.isBlank() } ?: "gpm",
        subtitle = subtitle?.takeUnless { it.isBlank() },
        negative = negative?.takeUnless { it.isBlank() } ?: "Cancel",
    )

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

/** Argument for `store` / `storeBiometric`: the Base64 32-byte master key. The
 *  title/subtitle/negative fields carry localized prompt text; only
 *  `storeBiometric` uses them (the auth-free `store` does not prompt). The
 *  `slot` field (R064) selects the biometric target for `storeBiometric`
 *  ("vault" | "legacy"; defaults to legacy via [BiometricSlot.fromString]). */
@InvokeArg
class StoreArgs {
    lateinit var key: String
    var title: String? = null
    var subtitle: String? = null
    var negative: String? = null
    var slot: String? = null
}

/** `retrieveBiometric` carries no secret — the localized prompt text + the R064
 *  biometric `slot` ("vault" | "legacy"; defaults to legacy). */
@InvokeArg
class RetrieveBiometricArgs {
    var title: String? = null
    var subtitle: String? = null
    var negative: String? = null
    var slot: String? = null
}

/** `deleteBiometric` carries only the R064 biometric `slot` ("vault" | "legacy";
 *  defaults to legacy). */
@InvokeArg
class DeleteBiometricArgs {
    var slot: String? = null
}

/**
 * Stores the gpm at-rest master key in the Android Keystore under two policies:
 * an auth-free key (default) and a biometric-gated key (opt-in app-lock).
 *
 * The auth-free path is API 23+ (minSdk 24). The biometric-gated path is API
 * 30+ (Android 11): its key uses
 * [KeyGenParameterSpec.Builder.setUserAuthenticationParameters], so every
 * encrypt/decrypt requires a CryptoObject-bound STRONG biometric prompt.
 */
@TauriPlugin
class SecureKeystorePlugin(private val activity: Activity) : Plugin(activity) {

    // ── Shared helpers ───────────────────────────────────────────────────

    /** The host activity as a [FragmentActivity], required by [BiometricPrompt]. */
    private fun fragmentActivity(): FragmentActivity? = activity as? FragmentActivity

    /** The STRONG biometric authenticators bitmask — must match the key's
     *  `AUTH_BIOMETRIC_STRONG` or `retrieveBiometric` fails at prompt time. */
    private val strongAuthenticator: Int
        get() = BiometricManager.Authenticators.BIOMETRIC_STRONG

    private fun storeCipherData(prefs: SharedPreferences, iv: ByteArray, ciphertext: ByteArray) {
        val (ivB64, ctB64) = encodeBlob(iv, ciphertext)
        prefs.edit().apply {
            putString(PREF_IV, ivB64)
            putString(PREF_CT, ctB64)
        }.apply()
    }

    /** The stored (iv, ciphertext) pair for `prefs`, or null if nothing is sealed. */
    private fun readCipherData(prefs: SharedPreferences): Pair<ByteArray, ByteArray>? =
        decodeBlob(prefs.getString(PREF_IV, null), prefs.getString(PREF_CT, null))

    // ── Auth-free master key ─────────────────────────────────────────────

    private fun prefs(): SharedPreferences =
        activity.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    /**
     * Generate a fresh auth-free AES/GCM key, replacing any prior entry. A fresh
     * key on every `store` sidesteps a stale-alias trap; `store` is called once
     * (when the master key is first generated), so the churn is harmless.
     */
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
            // Auth-free: no setUserAuthenticationRequired, no auth params, and
            // NOT invalidated by biometric enrollment — so the at-rest store
            // never bricks on a fingerprint change.
            .build()
        keyGenerator.init(spec)
        keyGenerator.generateKey()
    }

    private fun loadKey(): SecretKey {
        val keyStore = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        return (keyStore.getEntry(KEY_ALIAS, null) as KeyStore.SecretKeyEntry).secretKey
    }

    /** A [Cipher] initialised for encryption with a fresh IV. */
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

    // ── Biometric-gated keys (legacy app-lock master + R064 vault) ───────

    private fun biometricPrefs(slot: BiometricSlot): SharedPreferences =
        activity.getSharedPreferences(slot.prefsName, Context.MODE_PRIVATE)

    /**
     * Generate a fresh biometric-gated AES/GCM key for [slot], replacing any
     * prior entry.
     *
     * A fresh key on every `storeBiometric` sidesteps the "alias exists but key
     * is invalidated" trap (a dead key keeps its alias), so re-enabling after a
     * biometric change just works. API 30+: per-use STRONG biometric auth.
     *
     * Deliberately `setInvalidatedByBiometricEnrollment(false)`: the key cannot
     * self-heal (unlike a passphrase), so adding a fingerprint must NOT brick
     * the whole store. The residual risk — removing ALL biometrics invalidates
     * the key — is the documented re-setup case for the opt-in gate.
     */
    @RequiresApi(Build.VERSION_CODES.R)
    private fun generateBiometricKey(slot: BiometricSlot) {
        val keyStore = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        if (keyStore.containsAlias(slot.alias)) {
            keyStore.deleteEntry(slot.alias)
        }
        val keyGenerator =
            KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
        val spec = KeyGenParameterSpec.Builder(
            slot.alias,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
        )
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setUserAuthenticationRequired(true)
            // API 30+: every use requires a CryptoObject-bound STRONG biometric.
            .setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG)
            // Survive fingerprint enrollment (do NOT brick the store on a new
            // finger). Removing all biometrics invalidates or renders the key
            // unusable (KeyStore-behavior-dependent) → documented re-setup.
            .setInvalidatedByBiometricEnrollment(false)
            .build()
        keyGenerator.init(spec)
        keyGenerator.generateKey()
    }

    @RequiresApi(Build.VERSION_CODES.R)
    private fun loadBiometricKey(slot: BiometricSlot): SecretKey {
        val keyStore = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        return (keyStore.getEntry(slot.alias, null) as KeyStore.SecretKeyEntry).secretKey
    }

    /** A [Cipher] initialised for [slot]'s biometric-key encryption with a fresh IV. */
    @RequiresApi(Build.VERSION_CODES.R)
    private fun biometricEncryptionCipher(slot: BiometricSlot): Cipher {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, loadBiometricKey(slot))
        return cipher
    }

    /** A [Cipher] initialised for [slot]'s biometric-key decryption with the stored IV. */
    @RequiresApi(Build.VERSION_CODES.R)
    private fun biometricDecryptionCipher(slot: BiometricSlot, iv: ByteArray): Cipher {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, loadBiometricKey(slot), GCMParameterSpec(GCM_TAG_BITS, iv))
        return cipher
    }

    // ── Biometric prompt plumbing (mirrors KeystorePlugin.kt) ────────────

    private fun promptInfo(title: String?, subtitle: String?, negative: String?): BiometricPrompt.PromptInfo {
        val r = resolvePromptText(title, subtitle, negative)
        val builder = BiometricPrompt.PromptInfo.Builder()
            .setTitle(r.title)
            .setNegativeButtonText(r.negative)
            .setAllowedAuthenticators(strongAuthenticator)
        if (r.subtitle != null) builder.setSubtitle(r.subtitle)
        return builder.build()
    }

    // ── @Command surface: auth-free master key ───────────────────────────

    /** `true` — the Android Keystore is always present on Android. */
    @Command
    fun isAvailable(invoke: Invoke) {
        val ret = JSObject()
        ret.put("available", true)
        invoke.resolve(ret)
    }

    /**
     * Retrieve the sealed master key. Resolves `{ stored: false }` if nothing
     * is sealed, or `{ stored: true, key: <base64> }` otherwise. Non-prompting.
     */
    @Command
    fun retrieve(invoke: Invoke) {
        val (iv, ciphertext) = readCipherData(prefs()) ?: run {
            val ret = JSObject()
            ret.put("stored", false)
            invoke.resolve(ret)
            return
        }

        val plain = try {
            decryptionCipher(iv).doFinal(ciphertext)
        } catch (e: Exception) {
            invoke.reject(safeName(e), "SECURE_KEYSTORE_FAILED")
            return
        }

        val ret = JSObject()
        ret.put("stored", true)
        ret.put("key", Base64.encodeToString(plain, Base64.NO_WRAP))
        invoke.resolve(ret)
        plain.fill(0)
    }

    /** Seal the supplied Base64 master key into the auth-free Keystore. */
    @Command
    fun store(invoke: Invoke) {
        val keyB64 = invoke.parseArgs(StoreArgs::class.java).key
        val plain = try {
            Base64.decode(keyB64, Base64.NO_WRAP)
        } catch (e: Exception) {
            invoke.reject(safeName(e), "SECURE_KEYSTORE_FAILED")
            return
        }

        try {
            generateKey()
            val cipher = encryptionCipher()
            val ciphertext = cipher.doFinal(plain)
            storeCipherData(prefs(), cipher.iv, ciphertext)
        } catch (e: Exception) {
            invoke.reject(safeName(e), "SECURE_KEYSTORE_FAILED")
            return
        } finally {
            plain.fill(0)
        }
        // No-arg resolve() sends the literal "null", which the Rust handle
        // deserializes into `()`. `resolve(JSObject())` would send "{}", which
        // fails that `()` deserialize (invalid type: map, expected unit).
        invoke.resolve()
    }

    /** Delete the auth-free Keystore key and ciphertext (best-effort). */
    @Command
    fun delete(invoke: Invoke) {
        try {
            val keyStore = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
            if (keyStore.containsAlias(KEY_ALIAS)) {
                keyStore.deleteEntry(KEY_ALIAS)
            }
        } catch (_: Exception) {
            // Best-effort: still clear prefs so the app can always reset.
        }
        prefs().edit().clear().apply()
        invoke.resolve()
    }

    // ── @Command surface: biometric-gated master key (app-lock) ──────────

    /** Availability state for the biometric-gated master key — one of
     *  "available", "no_enrollment", "weak_enrolled", "unavailable". Non-
     *  prompting. Pre-API-30 → "unavailable" (the STRONG keystore key requires
     *  R). See [mapBiometricState] for the mapping. */
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

    /** `true` iff a sealed biometric key exists in EITHER slot AND still inits
     *  cleanly. Non-prompting. A key invalidated by all-biometrics-removed →
     *  false, so a cold launch skips a doomed prompt.
     *
     *  R064: checks BOTH the vault (`gpm_vault_key`) and the legacy
     *  (`gpm_master_key_biometric`) slots, so the "app-lock is on" signal stays
     *  correct across the m0007 transition — pre-m0007 upgraders hold the
     *  legacy alias (vault absent), post-m0007 hold the vault (legacy deleted). */
    @Command
    fun hasStoredBiometric(invoke: Invoke) {
        // Explicit API guard BEFORE any reference to the R-only probe, so the
        // method is robust to reordering (the `@RequiresApi(R)` body must never
        // be reached on API <30, or it would verify-error at first touch).
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) {
            val ret = JSObject()
            ret.put("stored", false)
            invoke.resolve(ret)
            return
        }
        val stored = hasUsableBiometricInAnySlot(
            legacyPresent = readCipherData(biometricPrefs(BiometricSlot.LEGACY)) != null,
            legacyUsable = biometricKeyUsable(BiometricSlot.LEGACY),
            vaultPresent = readCipherData(biometricPrefs(BiometricSlot.VAULT)) != null,
            vaultUsable = biometricKeyUsable(BiometricSlot.VAULT),
        )
        val ret = JSObject()
        ret.put("stored", stored)
        invoke.resolve(ret)
    }

    /** Whether [slot]'s biometric key still inits — i.e. is usable. Non-prompting:
     *  init on an authentication-bound key does NOT require auth; only the
     *  prompt does. Any init failure ⇒ not usable ⇒ fall back / re-setup.
     *
     *  Keep this body free of API-30-only symbols beyond the cipher init: the
     *  SDK guard in [hasStoredBiometric] gatekeeps, and `@RequiresApi(R)` is
     *  lint-only, not runtime-enforced. */
    @RequiresApi(Build.VERSION_CODES.R)
    private fun biometricKeyUsable(slot: BiometricSlot): Boolean = try {
        biometricEncryptionCipher(slot)
        true
    } catch (e: Exception) {
        Log.w("gpm_secure_keystore", "hasStoredBiometric probe failed (${slot.alias}): ${safeName(e)}")
        false
    }

    /**
     * Seal the supplied Base64 master key behind biometric auth. Shows a
     * CryptoObject ENCRYPT prompt (a `setUserAuthenticationRequired` key needs
     * auth for encrypt too) and resolves ONLY from a terminal callback.
     */
    @RequiresApi(Build.VERSION_CODES.R)
    @Command
    fun storeBiometric(invoke: Invoke) {
        val fa = fragmentActivity() ?: run {
            invoke.reject("not FragmentActivity", "BIOMETRIC_UNAVAILABLE")
            return
        }

        val args = invoke.parseArgs(StoreArgs::class.java)
        val keyB64 = args.key
        val slot = BiometricSlot.fromString(args.slot)
        val plain = try {
            Base64.decode(keyB64, Base64.NO_WRAP)
        } catch (e: Exception) {
            invoke.reject(safeName(e), "BIOMETRIC_FAILED")
            return
        }

        val cipher = try {
            generateBiometricKey(slot)
            biometricEncryptionCipher(slot)
        } catch (e: Exception) {
            plain.fill(0)
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
                        val ciphertext = authCipher.doFinal(plain)
                        storeCipherData(biometricPrefs(slot), authCipher.iv, ciphertext)
                        ciphertext.fill(0)
                        invoke.resolve()
                    } catch (e: Exception) {
                        invoke.reject(safeName(e), "BIOMETRIC_FAILED")
                    } finally {
                        plain.fill(0)
                    }
                }

                override fun onAuthenticationError(code: Int, errString: CharSequence) {
                    // Terminal (cancel / lockout / hw-unavailable) — the most
                    // common exit when enabling app lock. Zero the master-key
                    // bytes: the success path's `finally` doesn't run here.
                    plain.fill(0)
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
     * Retrieve the sealed master key behind biometric auth. Shows a CryptoObject
     * DECRYPT prompt and resolves ONLY from a terminal callback with
     * `{ stored: true, key: <base64> }`. A permanently-invalidated key
     * (all biometrics removed) maps to `BIOMETRIC_KEY_INVALIDATED`.
     */
    @Command
    fun retrieveBiometric(invoke: Invoke) {
        // API guard FIRST: the R-only cipher helpers must never be touched on
        // API <30. (Robust to reordering; the app layer also gates on
        // isBiometricAvailable, but this is the in-plugin backstop.)
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) {
            invoke.reject("biometric requires API 30+", "BIOMETRIC_UNAVAILABLE")
            return
        }

        val fa = fragmentActivity() ?: run {
            invoke.reject("not FragmentActivity", "BIOMETRIC_UNAVAILABLE")
            return
        }
        val args = invoke.parseArgs(RetrieveBiometricArgs::class.java)
        val slot = BiometricSlot.fromString(args.slot)

        val (iv, ciphertext) = readCipherData(biometricPrefs(slot)) ?: run {
            invoke.reject("nothing stored", "BIOMETRIC_NOT_SET")
            return
        }

        val cipher = try {
            biometricDecryptionCipher(slot, iv)
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
                        ret.put("stored", true)
                        ret.put("key", Base64.encodeToString(plain, Base64.NO_WRAP))
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

    /** Delete the biometric Keystore key + ciphertext for [slot] (best-effort).
     *
     *  R064: `slot` selects vault (`gpm_vault_key`) vs legacy
     *  (`gpm_master_key_biometric`); defaults to legacy. */
    @Command
    fun deleteBiometric(invoke: Invoke) {
        val slot = BiometricSlot.fromString(invoke.parseArgs(DeleteBiometricArgs::class.java).slot)
        try {
            val keyStore = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
            if (keyStore.containsAlias(slot.alias)) {
                keyStore.deleteEntry(slot.alias)
            }
        } catch (_: Exception) {
            // Best-effort: still clear prefs so the app can always reset.
        }
        biometricPrefs(slot).edit().clear().apply()
        invoke.resolve()
    }
}
