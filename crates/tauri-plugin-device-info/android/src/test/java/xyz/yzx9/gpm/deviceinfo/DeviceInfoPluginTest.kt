// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

package xyz.yzx9.gpm.deviceinfo

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Characterization tests for [DeviceFacts.toJson] — the shaping the Rust side
 * deserializes into `DeviceInfo`. Exercises the full-field case and the
 * null-metric / null-userAgent degradation (display collapses to null).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class DeviceInfoPluginTest {

    @Test
    fun toJson_shapesAllFields() {
        val facts = DeviceFacts(
            manufacturer = "Google",
            model = "Pixel 8",
            brand = "google",
            sdkInt = 34,
            release = "15",
            abis = listOf("arm64-v8a", "armeabi-v7a"),
            userAgent = "Mozilla/5.0 gpm",
            widthPx = 1080,
            heightPx = 2400,
            densityDpi = 420,
        )
        val json = facts.toJson()

        assertEquals("Google", json.optString("manufacturer"))
        assertEquals("Pixel 8", json.optString("model"))
        assertEquals("google", json.optString("brand"))
        assertEquals(34, json.optInt("sdk_int"))
        assertEquals("15", json.optString("release"))
        assertFalse("user_agent present", json.isNull("user_agent"))
        assertEquals("Mozilla/5.0 gpm", json.optString("user_agent"))

        val abis = json.optJSONArray("abis")!!
        assertEquals(2, abis.length())
        assertEquals("arm64-v8a", abis.getString(0))
        assertEquals("armeabi-v7a", abis.getString(1))

        val display = json.optJSONObject("display")!!
        assertEquals(1080, display.getInt("width_px"))
        assertEquals(2400, display.getInt("height_px"))
        assertEquals(420, display.getInt("density_dpi"))
    }

    @Test
    fun toJson_displayCollapsesToNullWhenAnyMetricMissing() {
        val facts = DeviceFacts(
            manufacturer = "Google",
            model = "Pixel",
            brand = "google",
            sdkInt = 34,
            release = "15",
            abis = emptyList(),
            userAgent = null,
            widthPx = null, // any missing metric => no display object
            heightPx = 2400,
            densityDpi = 420,
        )
        val json = facts.toJson()

        assertNull("display null when a metric is missing", json.optJSONObject("display"))
        assertTrue("user_agent null", json.isNull("user_agent"))
        val abis = json.optJSONArray("abis")!!
        assertEquals(0, abis.length())
    }
}
