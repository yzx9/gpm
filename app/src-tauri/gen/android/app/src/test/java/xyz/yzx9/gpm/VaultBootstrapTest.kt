// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package xyz.yzx9.gpm

import android.content.Context
import androidx.biometric.BiometricManager
import androidx.test.core.app.ApplicationProvider
import java.security.KeyStore
import javax.crypto.KeyGenerator
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Pins [VaultBootstrap]'s null branches and prompt posture — the
 * HeadlessBootstrapTest pattern: the `(KeyStore, SharedPreferences)` seam
 * driven with an ordinary **JCEKS** keystore (AndroidKeyStore doesn't exist
 * under Robolectric; the prompt/doFinal path needs real hardware and is
 * covered by the device smoke).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class VaultBootstrapTest {

    private val ctx = ApplicationProvider.getApplicationContext<Context>()

    private val prefs
        get() = ctx.getSharedPreferences(VaultBootstrap.PREFS_NAME, Context.MODE_PRIVATE)

    private fun emptyKeyStore(): KeyStore =
        KeyStore.getInstance("JCEKS").apply { load(null, null) }

    private fun keyStoreWithAlias(): KeyStore =
        KeyStore.getInstance("JCEKS").apply {
            load(null, null)
            val key = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()
            setEntry(
                VaultBootstrap.KEY_ALIAS,
                KeyStore.SecretKeyEntry(key),
                KeyStore.PasswordProtection(CharArray(0)),
            )
        }

    @Before
    fun clearPrefs() {
        prefs.edit().clear().apply()
    }

    @Test
    fun sealedBlobNullWhenAliasMissing() {
        assertNull(VaultBootstrap.sealedBlob(emptyKeyStore(), prefs))
    }

    @Test
    fun sealedBlobNullWhenIvMissing() {
        assertNull(VaultBootstrap.sealedBlob(keyStoreWithAlias(), prefs))
    }

    @Test
    fun sealedBlobNullWhenCtMissing() {
        prefs.edit().putString(VaultBootstrap.PREF_IV, "AAAA").apply()
        assertNull(VaultBootstrap.sealedBlob(keyStoreWithAlias(), prefs))
    }

    @Test
    fun sealedBlobPresentWhenAllSet() {
        prefs.edit()
            .putString(VaultBootstrap.PREF_IV, "AAAA")
            .putString(VaultBootstrap.PREF_CT, "BBBB")
            .apply()
        val (iv, ct) = VaultBootstrap.sealedBlob(keyStoreWithAlias(), prefs)!!
        // Base64.NO_WRAP round trip — opaque bytes, no real crypto needed.
        // "AAAA"/"BBBB" are 4 base64 chars → 3 bytes each.
        assertEquals(3, iv.size)
        assertEquals(3, ct.size)
    }

    @Test
    fun isVaultSealedTracksPrefs() {
        assertFalse(VaultBootstrap.isVaultSealed(ctx))
        prefs.edit()
            .putString(VaultBootstrap.PREF_IV, "AAAA")
            .putString(VaultBootstrap.PREF_CT, "BBBB")
            .apply()
        assertTrue(VaultBootstrap.isVaultSealed(ctx))
    }

    @Test
    fun promptInfoIsStrongOnlyWithFallbackStrings() {
        val info = VaultBootstrap.promptInfo()
        // The builder's getters exist from androidx.biometric 1.1.0.
        assertEquals("gpm", info.title)
        assertEquals("Cancel", info.negativeButtonText)
    }

    @Test
    fun strongBiometricAvailableOnlyOnSuccess() {
        assertTrue(VaultBootstrap.strongBiometricAvailable(BiometricManager.BIOMETRIC_SUCCESS))
        assertFalse(VaultBootstrap.strongBiometricAvailable(BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED))
        assertFalse(VaultBootstrap.strongBiometricAvailable(BiometricManager.BIOMETRIC_ERROR_NO_HARDWARE))
    }
}
