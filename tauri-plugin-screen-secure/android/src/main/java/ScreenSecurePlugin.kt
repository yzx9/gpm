// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

package xyz.yzx9.gpm.screensecure

import android.app.Activity
import android.view.WindowManager
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

/** Argument holder for `setSecure` (deserialized by field name). */
@InvokeArg
class SetSecureArgs {
    var secure: Boolean = false
}

/**
 * Toggles `WindowManager.LayoutParams.FLAG_SECURE` on the host activity's window.
 *
 * `secure = true`  → block screenshots, screen recording, and the Recents thumbnail.
 * `secure = false` → allow capture. The frontend reconciles the flag per-route on
 * navigation (see `useSecureScreen` / the `beforeEach` guard in `main.ts`).
 *
 * The shape mirrors `SafeAreaPlugin`; the `@InvokeArg` + `parseArgs` pattern mirrors
 * `KeystorePlugin`.
 */
@TauriPlugin
class ScreenSecurePlugin(private val activity: Activity) : Plugin(activity) {

    @Command
    fun setSecure(invoke: Invoke) {
        val secure = invoke.parseArgs(SetSecureArgs::class.java).secure
        if (secure) {
            activity.window.setFlags(
                WindowManager.LayoutParams.FLAG_SECURE,
                WindowManager.LayoutParams.FLAG_SECURE,
            )
        } else {
            activity.window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
        }
        invoke.resolve(JSObject())
    }
}
