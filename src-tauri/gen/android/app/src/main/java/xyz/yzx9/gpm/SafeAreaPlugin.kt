// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

package xyz.yzx9.gpm

import android.app.Activity
import android.webkit.WebView
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@TauriPlugin
class SafeAreaPlugin(private val activity: Activity) : Plugin(activity) {

    private var statusBarInset = 0.0
    private var navBarInset = 0.0

    override fun load(webView: WebView) {
        val density = activity.resources.displayMetrics.density
        val decorView = activity.window.decorView

        ViewCompat.setOnApplyWindowInsetsListener(decorView) { _, insets ->
            statusBarInset =
                insets.getInsets(WindowInsetsCompat.Type.statusBars()).top / density
            navBarInset =
                insets.getInsets(WindowInsetsCompat.Type.navigationBars()).bottom / density

            if (hasListener("safe-area-changed")) {
                val payload = JSObject()
                payload.put("top", statusBarInset)
                payload.put("bottom", navBarInset)
                trigger("safe-area-changed", payload)
            }
            insets
        }

        decorView.post {
            ViewCompat.requestApplyInsets(decorView)
        }
    }

    @Command
    fun get_insets(invoke: Invoke) {
        val ret = JSObject()
        ret.put("top", statusBarInset)
        ret.put("bottom", navBarInset)
        invoke.resolve(ret)
    }
}
