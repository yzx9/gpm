// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

package xyz.yzx9.gpm.clipboardnotify

import android.app.PendingIntent
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
import org.robolectric.Shadows.shadowOf
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
 * points is de-prioritized).
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

    @Test
    fun appNotificationSettingsIntent_carriesActionAndPackage() {
        // Pins the recovery deep-link target: the per-app notification settings
        // action + the app's own package as the extra.
        val intent = appNotificationSettingsIntent("xyz.yzx9.gpm")
        assertEquals(
            android.provider.Settings.ACTION_APP_NOTIFICATION_SETTINGS,
            intent.action,
        )
        assertEquals(
            "xyz.yzx9.gpm",
            intent.getStringExtra(android.provider.Settings.EXTRA_APP_PACKAGE),
        )
    }

    // broadcastPendingIntentFlags — pins the immutable-PendingIntent posture:
    // the tap broadcast the app fully owns must never be mutable. The function
    // branches on the passed-in sdkInt, so both sides of the gate are reached
    // despite the class's @Config(sdk = [34]).
    @Test
    fun broadcastPendingIntentFlags_preMarshmallow_hasNoImmutabilityBit() {
        val flags = broadcastPendingIntentFlags(Build.VERSION_CODES.LOLLIPOP_MR1)
        assertEquals(PendingIntent.FLAG_UPDATE_CURRENT, flags)
    }

    @Test
    fun broadcastPendingIntentFlags_marshmallowAndLater_isImmutable() {
        val expected = PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        assertEquals(expected, broadcastPendingIntentFlags(Build.VERSION_CODES.M))
        assertEquals(expected, broadcastPendingIntentFlags(Build.VERSION_CODES.UPSIDE_DOWN_CAKE))
    }

    // buildClearBroadcastPendingIntent — integration check that the helper's flag
    // bits actually reach getBroadcast(...), so a future edit that bypasses the
    // helper at the wiring site cannot silently make the tap PendingIntent
    // mutable. Robolectric's ShadowPendingIntent records the flags the
    // PendingIntent was built with; the real PendingIntent has no flag getter.
    @Test
    fun buildClearBroadcastPendingIntent_carriesImmutableFlagOnModernApi() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val pi = buildClearBroadcastPendingIntent(context, Build.VERSION_CODES.UPSIDE_DOWN_CAKE)
        val flags = shadowOf(pi).flags
        assertTrue(
            "the tap PendingIntent must carry FLAG_IMMUTABLE on API 23+",
            flags and PendingIntent.FLAG_IMMUTABLE != 0,
        )
    }
}
