// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package xyz.yzx9.gpm

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import java.security.KeyStore
import javax.crypto.KeyGenerator
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Pins [HeadlessBootstrap.loadAuthFreeMasterKey]'s null-return branches — the
 * "store not set up yet" common case where the worker must skip (return `null`)
 * rather than error: the auth-free alias missing, or the sealed iv/ct absent
 * from prefs.
 *
 * Drives the internal `(KeyStore, SharedPreferences)` seam with an ordinary
 * **JCEKS** keystore (the AndroidKeyStore provider doesn't exist under
 * Robolectric, and the decrypt path needs a real hardware-backed key anyway —
 * that path is covered by the device smoke, not here).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class HeadlessBootstrapTest {

    private val ctx = ApplicationProvider.getApplicationContext<Context>()

    private val prefs
        get() = ctx.getSharedPreferences(HeadlessBootstrap.PREFS_NAME, Context.MODE_PRIVATE)

    private fun emptyKeyStore(): KeyStore =
        KeyStore.getInstance("JCEKS").apply { load(null, null) }

    private fun keyStoreWithAlias(): KeyStore =
        KeyStore.getInstance("JCEKS").apply {
            load(null, null)
            val key = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()
            setEntry(
                HeadlessBootstrap.KEY_ALIAS,
                KeyStore.SecretKeyEntry(key),
                KeyStore.PasswordProtection(CharArray(0)),
            )
        }

    @Before
    fun clearPrefs() {
        prefs.edit().clear().apply()
    }

    @Test
    fun returnsNullWhenAliasMissing() {
        // No auth-free key alias ⇒ store not set up yet ⇒ skip.
        assertNull(HeadlessBootstrap.loadAuthFreeMasterKey(emptyKeyStore(), prefs))
    }

    @Test
    fun returnsNullWhenIvMissing() {
        // Alias present, prefs empty ⇒ no iv ⇒ null before touching the key.
        assertNull(HeadlessBootstrap.loadAuthFreeMasterKey(keyStoreWithAlias(), prefs))
    }

    @Test
    fun returnsNullWhenCtMissing() {
        // Alias present, iv present, ct absent ⇒ null.
        prefs.edit().putString(HeadlessBootstrap.PREF_IV, "AAAA").apply()
        assertNull(HeadlessBootstrap.loadAuthFreeMasterKey(keyStoreWithAlias(), prefs))
    }
}
