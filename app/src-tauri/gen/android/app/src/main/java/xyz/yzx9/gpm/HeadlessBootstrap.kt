// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package xyz.yzx9.gpm

import android.content.Context
import android.content.SharedPreferences
import android.util.Base64
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.spec.GCMParameterSpec

/**
 * Retrieves the **auth-free** at-rest master key (the one that seals the
 * metadata — `repo.json` / `app.json`; since R064 the `identity` lives under a
 * separate `gpm_vault_key`, not this key) directly from the Android Keystore —
 * background-safe, no biometric prompt (the auth-free key has no
 * `setUserAuthenticationRequired`).
 *
 * App-owned headless bootstrap (R077): the OS-started entry points (the
 * WorkManager `SyncWorker`, a future Autofill service) have no Tauri
 * `AppHandle`, so they cannot call the keystore plugin's `@Command`. This is
 * the auth-free retrieve they share. `KEY_ALIAS`/`PREFS_NAME` are the Kotlin
 * mirrors of the Rust consts `MASTER_ALIAS`/`MASTER_PREFS` in
 * `app/src-tauri/src/keystore.rs` — keep them in sync on rename (the
 * duplication is inherent: Kotlin can't read Rust consts).
 */
object HeadlessBootstrap {
    private const val ANDROID_KEYSTORE = "AndroidKeyStore"
    internal const val KEY_ALIAS = "gpm_master_key"
    internal const val PREFS_NAME = "gpm_secure_keystore"
    internal const val PREF_IV = "iv"
    internal const val PREF_CT = "ct"
    private const val GCM_TAG_BITS = 128

    /**
     * The auth-free master key (base64, STANDARD — matches Rust's `decode_master_key`),
     * or `null` if the auth-free store is empty/unset. Never prompts.
     *
     * R064: the auth-free `gpm_master_key` is permanent (never deleted on App Lock
     * toggle), so a background worker retrieves it under App Lock and syncs.
     */
    fun loadAuthFreeMasterKey(context: Context): String? {
        val keyStore = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        return loadAuthFreeMasterKey(
            keyStore,
            context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE),
        )
    }

    /**
     * Testable seam: the auth-free retrieve against an explicit [KeyStore] +
     * [SharedPreferences] pair (the two OS collaborators the public entry
     * resolves from a [Context]). Split so the null-return branches run under
     * JVM/Robolectric with an ordinary keystore — the decrypt path itself needs
     * a real hardware-backed key and is covered by the device smoke.
     */
    internal fun loadAuthFreeMasterKey(keyStore: KeyStore, prefs: SharedPreferences): String? {
        if (!keyStore.containsAlias(KEY_ALIAS)) return null

        val ivB64 = prefs.getString(PREF_IV, null) ?: return null
        val ctB64 = prefs.getString(PREF_CT, null) ?: return null

        val iv = Base64.decode(ivB64, Base64.NO_WRAP)
        val ct = Base64.decode(ctB64, Base64.NO_WRAP)
        val key = (keyStore.getEntry(KEY_ALIAS, null) as KeyStore.SecretKeyEntry).secretKey
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(GCM_TAG_BITS, iv))
        val plain = cipher.doFinal(ct)
        return Base64.encodeToString(plain, Base64.NO_WRAP)
    }
}
