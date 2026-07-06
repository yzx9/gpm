// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

package xyz.yzx9.gpm.clipboardnotify

import android.content.BroadcastReceiver
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import androidx.core.app.NotificationManagerCompat

/**
 * Handles the notification's body tap: clears the clipboard, dismisses the
 * sticky notification, and sets the manual-clear flag. Everything here uses
 * only the [Context] from [onReceive] — no Activity, no WebView, no
 * foregrounding, no plugin reference.
 *
 * Dynamically registered by [ClipboardNotifyPlugin.load], so it lives for the
 * app process lifetime (the only window in which the Rust timer matters). The
 * flag is consumed by Rust's armed clear timer on wake (via
 * `consumeManualClearFlag`), so the timer self-skips instead of clobbering
 * unrelated clipboard content the user placed after this tap.
 *
 * Default no-arg constructor for safety (Android may instantiate receivers via
 * reflection even when dynamically registered).
 */
class ClipboardClearReceiver : BroadcastReceiver() {

    override fun onReceive(context: Context, intent: Intent) {
        // Clear the clipboard natively. Background `setPrimaryClip` is allowed
        // (Android 10+ restricts background READS, not writes).
        val cm = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        cm.setPrimaryClip(ClipData.newPlainText("", ""))

        // Dismiss the sticky notification.
        NotificationManagerCompat.from(context).cancel(NOTIFICATION_ID)

        // Set the manual-clear flag so the Rust armed timer self-skips on wake
        // (it would otherwise fire later and clobber whatever the user copies
        // next). Consumed + reset by `consumeManualClearFlag` on the Rust side.
        context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
            .edit().putBoolean(KEY_MANUALLY_CLEARED, true).apply()
    }
}
