// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

package xyz.yzx9.gpm.backgroundsync

import android.content.Context
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
 * SELF-CONTAINED DUPLICATE of `tauri-plugin-secure-keystore`'s auth-free
 * retrieve path. **Keep in sync with `SecureKeystorePlugin.kt`** (alias /
 * provider / cipher / prefs names). The D3 plan called for a shared util module
 * across the two plugins, but the cross-plugin Gradle dependency is unproven
 * under Tauri's composite-build setup; promote to a shared module once that
 * wiring is verified (and `SecureKeystorePlugin` is refactored to call it too).
 */
object MasterKeyAccess {
    private const val ANDROID_KEYSTORE = "AndroidKeyStore"
    private const val KEY_ALIAS = "gpm_master_key"
    private const val PREFS_NAME = "gpm_secure_keystore"
    private const val PREF_CT = "ct"
    private const val PREF_IV = "iv"
    private const val GCM_TAG_BITS = 128

    /**
     * The auth-free master key (base64, STANDARD — matches Rust's `decode_master_key`),
     * or `null` if the auth-free store is empty/unset. Never prompts.
     *
     * R064: the auth-free `gpm_master_key` is permanent (never deleted on App Lock
     * toggle), so a background worker retrieves it under App Lock and syncs. The
     * old "AppLock on ⇒ biometric alias exists ⇒ skip" guard is removed — the
     * identity now lives under a separate `gpm_vault_key`, not this auth-free key.
     */
    fun loadAuthFree(context: Context): String? {
        val keyStore = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        if (!keyStore.containsAlias(KEY_ALIAS)) return null

        val prefs = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
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
