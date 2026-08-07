// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Backend-only device-info probe for gpm's diagnostics export: snapshots the
// Android hardware/OS build fields, the WebView user-agent, and the display
// metrics and returns them to Rust. Nothing secret is read. The frontend never
// calls this directly.

package xyz.yzx9.gpm.deviceinfo

import android.app.Activity
import android.os.Build
import android.util.DisplayMetrics
import android.webkit.WebSettings
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

/** Snapshot of the device facts gathered for diagnostics, decoupled from
 *  android.os.Build so the shaping is unit-testable without shadowing Build. */
internal data class DeviceFacts(
    val manufacturer: String?,
    val model: String?,
    val brand: String?,
    val sdkInt: Int?,
    val release: String?,
    val abis: List<String>,
    val userAgent: String?,
    val widthPx: Int?,
    val heightPx: Int?,
    val densityDpi: Int?,
)

/** Shape [DeviceFacts] into the JSObject Rust deserializes into `DeviceInfo`.
 *  Pure (no Build access) so it is unit-testable. The nested `display` object is
 *  present only when all three metrics are available, otherwise null (Rust reads
 *  it as `Option`). */
internal fun DeviceFacts.toJson(): JSObject {
    val display: JSObject? = if (widthPx != null && heightPx != null && densityDpi != null) {
        JSObject().apply {
            put("width_px", widthPx)
            put("height_px", heightPx)
            put("density_dpi", densityDpi)
        }
    } else {
        null
    }
    return JSObject().apply {
        put("manufacturer", manufacturer)
        put("model", model)
        put("brand", brand)
        put("sdk_int", sdkInt)
        put("release", release)
        put("abis", JSArray.from(abis.toTypedArray()))
        put("user_agent", userAgent)
        put("display", display)
    }
}

/**
 * Backend-only device-info probe. Registered from Rust via
 * `register_android_plugin("xyz.yzx9.gpm.deviceinfo", "DeviceInfoPlugin")`.
 */
@TauriPlugin
class DeviceInfoPlugin(private val activity: Activity) : Plugin(activity) {

    /** Snapshot the device facts and return them to Rust. */
    @Command
    fun device_info(invoke: Invoke) {
        val dm: DisplayMetrics = activity.resources.displayMetrics
        val facts = DeviceFacts(
            manufacturer = Build.MANUFACTURER,
            model = Build.MODEL,
            brand = Build.BRAND,
            sdkInt = Build.VERSION.SDK_INT,
            release = Build.VERSION.RELEASE,
            abis = Build.SUPPORTED_ABIS.toList(),
            userAgent = userAgent(),
            widthPx = dm.widthPixels,
            heightPx = dm.heightPixels,
            densityDpi = dm.densityDpi,
        )
        invoke.resolve(facts.toJson())
    }

    /** WebView user-agent. `getDefaultUserAgent` may throw on unusual builds, so
     *  degrade to null rather than failing the whole probe. */
    private fun userAgent(): String? = try {
        WebSettings.getDefaultUserAgent(activity)
    } catch (e: Throwable) {
        null
    }
}
