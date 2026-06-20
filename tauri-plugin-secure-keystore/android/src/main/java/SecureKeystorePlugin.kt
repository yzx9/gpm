// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0
//
// Auth-free Android Keystore storage for the gpm at-rest **master key**.
//
// This is the auth-free sibling of KeystorePlugin (biometric-keystore): same
// AndroidKeyStore AES/GCM mechanism, but the key is NOT user-authentication
// required and NOT invalidated by biometric enrollment, so there is no
// BiometricPrompt and the at-rest store survives fingerprint changes. The
// master key is a random 32-byte secret; the plugin seals it (iv + ciphertext
// in SharedPreferences) and hands the plaintext back to Rust (Base64 over
// IPC), exactly as KeystorePlugin hands back the passphrase.
//
// The crypto pattern is adapted from impierce/tauri-plugin-keystore
// (Apache-2.0), the reviewed reference used for the biometric plugin.

package xyz.yzx9.gpm.securekeystore

import android.app.Activity
import android.content.Context
import android.content.SharedPreferences
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
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
private const val KEY_ALIAS = "gpm_master_key"
private const val PREFS_NAME = "gpm_secure_keystore"
private const val PREF_CT = "ct"
private const val PREF_IV = "iv"

/** GCM authentication tag length, in bits. */
private const val GCM_TAG_BITS = 128

/** Argument for `store`: the Base64-encoded 32-byte master key. */
@InvokeArg
class StoreArgs {
    lateinit var key: String
}

/**
 * Stores the gpm at-rest master key in the Android Keystore, sealed with a
 * hardware-backed AES/GCM key that is **auth-free** (no biometric prompt, not
 * invalidated by fingerprint enrollment).
 *
 * The key is API 23+ (no biometric requirement); the plugin's minSdk is 24.
 */
@TauriPlugin
class SecureKeystorePlugin(private val activity: Activity) : Plugin(activity) {

    // ── Key + cipher management ──────────────────────────────────────────

    private fun prefs(): SharedPreferences =
        activity.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    /**
     * Generate a fresh auth-free AES/GCM key, replacing any prior entry.
     *
     * A fresh key on every `store` sidesteps a stale-alias trap and mirrors
     * the biometric plugin; `store` is called once (when the master key is
     * first generated), so the churn is harmless.
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

    /** Class name only — never leak crypto internals or secret data. */
    private fun safeName(e: Throwable): String = e.javaClass.simpleName.ifEmpty { "error" }

    // ── @Command surface ─────────────────────────────────────────────────

    /** `true` — the Android Keystore is always present on Android. */
    @Command
    fun is_available(invoke: Invoke) {
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
        val (iv, ciphertext) = readCipherData() ?: run {
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

    /** Seal the supplied Base64 master key into the Keystore. */
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
            storeCipherData(cipher.iv, ciphertext)
        } catch (e: Exception) {
            invoke.reject(safeName(e), "SECURE_KEYSTORE_FAILED")
            return
        } finally {
            plain.fill(0)
        }
        invoke.resolve(JSObject())
    }

    /** Delete the Keystore key and the stored ciphertext (best-effort). */
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
        invoke.resolve(JSObject())
    }
}
