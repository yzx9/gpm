// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0
//
// Sticky Android notification shown while a secret is on the clipboard. The
// notification's body tap fires an explicit-broadcast PendingIntent that the
// manifest-declared, `exported="false"` `ClipboardClearReceiver` handles
// natively: clear the clipboard, dismiss the notification, and set the
// "manually cleared" flag. The Rust armed clear timer consumes that flag on
// wake (run_mobile_plugin_async, the proven Rust→Kotlin direction) and skips
// its own clear if the user already cleared manually — so it cannot later
// clobber unrelated clipboard content the user placed after the tap (RFC 0037).
//
// The manifest receiver (vs. dynamic registration) means: (a) only our own
// PendingIntent can reach it on every Android version (no cross-app
// reachability like a pre-13 dynamic receiver would have), and (b) the tap
// still delivers after process death — the system instantiates the receiver.
//
// Backend-only: invoked from Rust via `run_mobile_plugin_*`, never from the
// WebView. The notification body is generic and the visibility is PRIVATE so
// no entry-name / secret-adjacent metadata leaks into the shade or the lock
// screen. Permission handling uses Tauri's declaration system
// (`@TauriPlugin permissions` + `requestPermissionForAlias` +
// `@PermissionCallback`), mirroring the official tauri-plugin-notification.

package xyz.yzx9.gpm.clipboardnotify

import android.app.Activity
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import android.content.SharedPreferences
import android.Manifest
import android.os.Build
import android.os.Build.VERSION.SDK_INT
import android.provider.Settings
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

internal const val NOTIFICATION_ID = 0xC1EA
internal const val CHANNEL_ID = "clipboard-clear"
internal const val PREFS_NAME = "gpm.clipboard.notify"
internal const val KEY_MANUALLY_CLEARED = "manually_cleared"
internal const val ALIAS_POST_NOTIFICATIONS = "postNotifications"

/** Reset the manual-clear flag. Called by `postClipboardNotification` BEFORE
 *  showing the notification (post always precedes any tap, so the receiver's
 *  tap-set can't race with a task-start reset). Pure over [SharedPreferences]. */
internal fun resetManualClearFlag(prefs: SharedPreferences) {
    prefs.edit().putBoolean(KEY_MANUALLY_CLEARED, false).apply()
}

/** Set the manual-clear flag. Called by `ClipboardClearReceiver` AFTER clearing
 *  the clipboard + dismissing the notification, so the armed timer self-skips on
 *  wake instead of clobbering content the user placed after the tap. */
internal fun setManualClearFlag(prefs: SharedPreferences) {
    prefs.edit().putBoolean(KEY_MANUALLY_CLEARED, true).apply()
}

/** Read-then-reset the manual-clear flag: returns whether a manual clear happened
 *  since the last reset (and resets it so). NOT transactionally atomic — it is a
 *  `getBoolean` followed by a conditional `apply` — but sufficient because Rust
 *  polls it once per wake on a single process. */
internal fun takeManualClearFlag(prefs: SharedPreferences): Boolean {
    val wasCleared = prefs.getBoolean(KEY_MANUALLY_CLEARED, false)
    if (wasCleared) {
        prefs.edit().putBoolean(KEY_MANUALLY_CLEARED, false).apply()
    }
    return wasCleared
}

/** Whether POST_NOTIFICATIONS must be requested at runtime: only on Android 13+
 *  (TIRAMISU) and only when not already granted. Pre-13 has no such permission. */
internal fun shouldRequestNotificationPermission(sdkInt: Int, notificationsEnabled: Boolean): Boolean =
    sdkInt >= Build.VERSION_CODES.TIRAMISU && !notificationsEnabled

/** PendingIntent flags for the manifest-receiver tap broadcast: always
 *  [PendingIntent.FLAG_UPDATE_CURRENT], plus [PendingIntent.FLAG_IMMUTABLE]
 *  on API 23+ (M) — a PendingIntent the app fully owns must never be mutable
 *  (mutable PendingIntents are a known Android escalation vector). Pre-M the
 *  immutability flag does not exist. Pure so the gating is Robolectric-testable. */
internal fun broadcastPendingIntentFlags(sdkInt: Int): Int {
    val base = android.app.PendingIntent.FLAG_UPDATE_CURRENT
    return if (sdkInt >= Build.VERSION_CODES.M) {
        base or android.app.PendingIntent.FLAG_IMMUTABLE
    } else {
        base
    }
}

/** Builds the manifest-receiver tap [android.app.PendingIntent]. Top-level
 *  (takes a [Context] rather than the plugin's activity) so the wiring — that
 *  [broadcastPendingIntentFlags]'s bits actually reach `getBroadcast(...)` — is
 *  Robolectric-testable without instantiating the Tauri plugin. The explicit
 *  class target + `exported=false` keeps other apps out and delivers the tap
 *  regardless of the receiver's export status. */
internal fun buildClearBroadcastPendingIntent(
    context: Context,
    sdkInt: Int,
): android.app.PendingIntent {
    val intent = Intent(context, ClipboardClearReceiver::class.java)
    return android.app.PendingIntent.getBroadcast(
        context,
        0,
        intent,
        broadcastPendingIntentFlags(sdkInt),
    )
}

/** Per-app notification-settings intent — the recovery surface when Android has
 *  suppressed the `POST_NOTIFICATIONS` runtime prompt after two denials (the only
 *  way back to re-enabling the clipboard-clear notification). Robolectric-testable
 *  so the action + `EXTRA_APP_PACKAGE` are pinned (the constants are compile-time
 *  inlined, but `Intent` itself needs the Android runtime, hence Robolectric). */
internal fun appNotificationSettingsIntent(packageName: String): Intent =
    Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
        .putExtra(Settings.EXTRA_APP_PACKAGE, packageName)

@InvokeArg
class PostClipboardNotificationArgs {
    /** Auto-clear window to advertise in the notification body, in seconds. */
    var secs: Long = 0
    /** Localized notification text; null ⇒ generic fallback. */
    var title: String? = null
    var body: String? = null
    var channelName: String? = null
    var channelDescription: String? = null
}

/**
 * Sticky clipboard-clear notification plugin.
 *
 * Registered from Rust via `register_android_plugin(
 * "xyz.yzx9.gpm.clipboardnotify", "ClipboardNotifyPlugin")` and invoked through
 * the `tauri-plugin-clipboard-notify` handle. The tap receiver is manifest-
 * declared (no dynamic registration).
 */
@TauriPlugin(
    permissions = [
        Permission(
            strings = [Manifest.permission.POST_NOTIFICATIONS],
            alias = ALIAS_POST_NOTIFICATIONS,
        ),
    ],
)
class ClipboardNotifyPlugin(private val activity: Activity) : Plugin(activity) {

    // ── commands ────────────────────────────────────────────────────────

    /** Whether the app may post notifications. Cheap, non-prompting. */
    @Command
    fun areNotificationsEnabled(invoke: Invoke) {
        resolveGranted(invoke, NotificationManagerCompat.from(activity).areNotificationsEnabled())
    }

    /**
     * Request POST_NOTIFICATIONS at runtime (Android 13+). On 13+ when not yet
     * granted, `requestPermissionForAlias` fires the system dialog and **holds
     * the Invoke across it**, routing the result to [permissionsCallback]; the
     * caller's `await` (Rust `run_mobile_plugin_async`) thereby blocks until
     * the user answers. Pre-13 (no POST_NOTIFICATIONS) and already-granted
     * cases resolve immediately.
     */
    @Command
    fun requestNotificationsPermission(invoke: Invoke) {
        if (!shouldRequestNotificationPermission(
                SDK_INT,
                NotificationManagerCompat.from(activity).areNotificationsEnabled(),
            )
        ) {
            resolveGranted(invoke, true)
            return
        }
        requestPermissionForAlias(ALIAS_POST_NOTIFICATIONS, invoke, "permissionsCallback")
    }

    /** Dialog-result callback — resolves the held Invoke with the grant state. */
    @PermissionCallback
    private fun permissionsCallback(invoke: Invoke) {
        resolveGranted(invoke, NotificationManagerCompat.from(activity).areNotificationsEnabled())
    }

    /**
     * Open the system's per-app notification-settings screen — the recovery
     * surface when the runtime `POST_NOTIFICATIONS` dialog is suppressed after
     * two denials (Android then stops re-prompting, so this is the only way back).
     * Fire-and-forget: resolves `{ opened: true }`, or `{ opened: false }` when no
     * activity can handle the intent (exotic OEM ROM) so the caller can toast
     * instead of failing silently. Any other throw rejects the invoke.
     */
    @Command
    fun openAppNotificationSettings(invoke: Invoke) {
        val opened = try {
            activity.startActivity(appNotificationSettingsIntent(activity.packageName))
            true
        } catch (_: android.content.ActivityNotFoundException) {
            false
        }
        val ret = JSObject()
        ret.put("opened", opened)
        invoke.resolve(ret)
    }

    /** Post (or update, by fixed ID) the sticky clipboard-clear notification. */
    @Command
    fun postClipboardNotification(invoke: Invoke) {
        val args = invoke.parseArgs(PostClipboardNotificationArgs::class.java)
        // Reset the manual-clear flag BEFORE showing the notification (post
        // always precedes any user tap — see `resetManualClearFlag`).
        resetManualClearFlag(activity.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE))
        ensureChannel(args.channelName, args.channelDescription)
        val notif =
            NotificationCompat.Builder(activity, CHANNEL_ID)
                .setSmallIcon(R.drawable.ic_clipboard_notify)
                .setContentTitle(args.title?.takeUnless { it.isBlank() } ?: "gpm")
                .setContentText(args.body?.takeUnless { it.isBlank() } ?: "Tap to clear")
                .setOngoing(true)
                .setAutoCancel(false)
                .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
                .setOnlyAlertOnce(true)
                .setPriority(NotificationCompat.PRIORITY_LOW)
                .setContentIntent(broadcastClearIntent())
                .build()
        try {
            NotificationManagerCompat.from(activity).notify(NOTIFICATION_ID, notif)
        } catch (_: SecurityException) {
            // Notification permission revoked between check and post — degrade silently.
        }
        // No-arg resolve() sends "null" → Rust `()`. resolve(JSObject()) sends
        // "{}", which fails the `()` deserialize (invalid type: map, expected unit).
        invoke.resolve()
    }

    /** Dismiss the sticky notification. */
    @Command
    fun dismissClipboardNotification(invoke: Invoke) {
        NotificationManagerCompat.from(activity).cancel(NOTIFICATION_ID)
        invoke.resolve()
    }

    /**
     * Atomically read + reset the manual-clear flag. Called by Rust on the
     * armed timer's wake (to detect a manual tap during the window). Returns
     * whether a manual clear happened since the last reset (which `postClipboardNotification`
     * does at post time).
     */
    @Command
    fun consumeManualClearFlag(invoke: Invoke) {
        val wasCleared =
            takeManualClearFlag(activity.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE))
        val ret = JSObject()
        ret.put("cleared", wasCleared)
        invoke.resolve(ret)
    }

    // ── helpers ─────────────────────────────────────────────────────────

    /**
     * Create the notification channel if absent. The localized name/description
     * are baked in at creation time: Android ignores
     * name changes on an existing channel, so a locale switch does NOT recreate
     * it (that would reset the user's per-channel settings) — the channel name
     * reflects the locale active at first creation. Generic fallbacks (NOT a
     * duplicate of native.json/en) when the frontend omits them.
     */
    private fun ensureChannel(channelName: String?, channelDescription: String?) {
        if (SDK_INT >= Build.VERSION_CODES.O) {
            val channel =
                NotificationChannel(
                    CHANNEL_ID,
                    channelName?.takeUnless { it.isBlank() } ?: "gpm",
                    NotificationManager.IMPORTANCE_LOW,
                ).apply {
                    description = channelDescription?.takeUnless { it.isBlank() } ?: "gpm"
                }
            NotificationManagerCompat.from(activity).createNotificationChannel(channel)
        }
    }

    private fun broadcastClearIntent(): android.app.PendingIntent =
        buildClearBroadcastPendingIntent(activity, SDK_INT)

    private fun resolveGranted(invoke: Invoke, granted: Boolean) {
        val ret = JSObject()
        ret.put("granted", granted)
        invoke.resolve(ret)
    }
}
