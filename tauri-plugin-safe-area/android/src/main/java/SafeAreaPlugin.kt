// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

package xyz.yzx9.gpm.safearea

import android.app.Activity
import android.webkit.WebView
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

/** Per-edge safe-area insets in CSS px (dp). */
internal data class SafeInsets(
    val top: Double,
    val bottom: Double,
    val left: Double,
    val right: Double,
)

/**
 * Per-edge safe-area insets (status bar + nav bar + display cutout) in CSS px.
 *
 * `Type.systemBars()` excludes the display cutout, so the cutout is ORed in
 * explicitly. `statusBars() or navigationBars() or displayCutout()` is used
 * rather than `systemBars()` to avoid bundling `captionBar()`, which can differ
 * from the nav bar on some OEM edge-to-edge builds. On cutout-free devices the
 * cutout inset is 0 on every edge, so this is a no-op there.
 *
 * Returns all-zero if no insets have been dispatched yet. [density] converts raw
 * px to CSS px. Pure — takes no Activity, so it is unit-testable in isolation.
 */
internal fun computeInsets(rootInsets: WindowInsetsCompat?, density: Float): SafeInsets {
    if (rootInsets == null) return SafeInsets(0.0, 0.0, 0.0, 0.0)
    val mask = WindowInsetsCompat.Type.statusBars() or
        WindowInsetsCompat.Type.navigationBars() or
        WindowInsetsCompat.Type.displayCutout()
    val insets = rootInsets.getInsets(mask)
    return SafeInsets(
        top = (insets.top / density).toDouble(),
        bottom = (insets.bottom / density).toDouble(),
        left = (insets.left / density).toDouble(),
        right = (insets.right / density).toDouble(),
    )
}

@TauriPlugin
class SafeAreaPlugin(private val activity: Activity) : Plugin(activity) {

    override fun load(webView: WebView) {
        val density = activity.resources.displayMetrics.density
        val decorView = activity.window.decorView

        ViewCompat.setOnApplyWindowInsetsListener(decorView) { _, insets ->
            // The JS resize/orientationchange re-pull is the reliable delivery
            // path for these values; this listener is best-effort (it doesn't
            // consistently fire in this edge-to-edge WebView).
            if (hasListener("safe-area-changed")) {
                trigger("safe-area-changed", computeInsets(insets, density).toJSObject())
            }
            insets
        }

        decorView.post {
            ViewCompat.requestApplyInsets(decorView)
        }
    }

    @Command
    fun get_insets(invoke: Invoke) {
        val density = activity.resources.displayMetrics.density
        val rootInsets = ViewCompat.getRootWindowInsets(activity.window.decorView)
        invoke.resolve(computeInsets(rootInsets, density).toJSObject())
    }
}

private fun SafeInsets.toJSObject(): JSObject {
    val ret = JSObject()
    ret.put("top", top)
    ret.put("bottom", bottom)
    ret.put("left", left)
    ret.put("right", right)
    return ret
}
