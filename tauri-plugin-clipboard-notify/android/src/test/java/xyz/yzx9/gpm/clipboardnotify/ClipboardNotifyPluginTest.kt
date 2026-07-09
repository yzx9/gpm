// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

package xyz.yzx9.gpm.clipboardnotify

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.os.Build
import androidx.test.core.app.ApplicationProvider
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Characterization tests for the clipboard-clear manual-clear invariant.
 *
 * The flag is a Boolean in SharedPreferences (survives process death; the tap
 * receiver is manifest-declared). `takeManualClearFlag` is read-then-reset — NOT
 * transactionally atomic, but sufficient because Rust polls it once per wake on a
 * single process. The receiver test drives `ClipboardClearReceiver.onReceive`
 * directly (a plain BroadcastReceiver — no Tauri runtime) to lock the clear+set
 * end state. Statement-level ordering (reset-before-notify, clear-before-set) is
 * enforced by code review, not unit-tested (driving the Tauri `@Command` entry
 * points is de-prioritized by RFC-0041).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class ClipboardNotifyPluginTest {

    private fun prefs() =
        ApplicationProvider.getApplicationContext<Context>()
            .getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)

    @Test
    fun flagState_resetClearsPriorTrueFlag() {
        // Set true first so the test fails if reset is a no-op (a reset on an
        // already-false flag would pass vacuously).
        setManualClearFlag(prefs())
        resetManualClearFlag(prefs())
        assertFalse(takeManualClearFlag(prefs()))
    }

    @Test
    fun flagState_setThenTakeReturnsTrueAndResets() {
        resetManualClearFlag(prefs())
        setManualClearFlag(prefs())
        assertTrue(takeManualClearFlag(prefs()))
        // takeManualClearFlag resets after reading — a second take returns false.
        assertFalse(takeManualClearFlag(prefs()))
    }

    @Test
    fun receiver_clearsClipboardAndSetsFlag() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        // Pre-seed the clipboard so the clear is observable.
        val cm = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        cm.setPrimaryClip(ClipData.newPlainText("label", "secret"))
        // Pre-reset the flag (as postClipboardNotification does at post time).
        resetManualClearFlag(context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE))

        ClipboardClearReceiver().onReceive(context, Intent())

        // The receiver cleared the clipboard — a non-null clip with empty text
        // (not empty-or-null, which would mask a regression to a null clip).
        val clip = cm.primaryClip
        assertNotNull(clip)
        assertEquals("", clip!!.getItemAt(0).text.toString())
        // … and set the manual-clear flag (so the armed timer self-skips on wake).
        assertTrue(
            context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
                .getBoolean(KEY_MANUALLY_CLEARED, false)
        )
    }

    @Test
    fun shouldRequestNotificationPermission_preTiramisuReturnsFalse() {
        assertFalse(shouldRequestNotificationPermission(Build.VERSION_CODES.S, true))
        assertFalse(shouldRequestNotificationPermission(Build.VERSION_CODES.S, false))
    }

    @Test
    fun shouldRequestNotificationPermission_tiramisuAndNotEnabledReturnsTrue() {
        assertTrue(shouldRequestNotificationPermission(Build.VERSION_CODES.TIRAMISU, false))
    }

    @Test
    fun shouldRequestNotificationPermission_tiramisuAndEnabledReturnsFalse() {
        assertFalse(shouldRequestNotificationPermission(Build.VERSION_CODES.TIRAMISU, true))
    }
}
