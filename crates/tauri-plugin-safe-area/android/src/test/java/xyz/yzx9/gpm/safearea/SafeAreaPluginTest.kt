// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

package xyz.yzx9.gpm.safearea

import androidx.core.graphics.Insets
import androidx.core.view.WindowInsetsCompat
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class SafeAreaPluginTest {

    /** Builds a WindowInsetsCompat with per-type insets; unset types are 0. */
    private fun insets(
        statusBars: Insets = Insets.NONE,
        navigationBars: Insets = Insets.NONE,
        displayCutout: Insets = Insets.NONE,
    ): WindowInsetsCompat =
        WindowInsetsCompat.Builder()
            .setInsets(WindowInsetsCompat.Type.statusBars(), statusBars)
            .setInsets(WindowInsetsCompat.Type.navigationBars(), navigationBars)
            .setInsets(WindowInsetsCompat.Type.displayCutout(), displayCutout)
            .build()

    @Test
    fun `null insets return all zeros`() {
        val si = computeInsets(null, 1f)
        assertEquals(0.0, si.top, 0.0)
        assertEquals(0.0, si.bottom, 0.0)
        assertEquals(0.0, si.left, 0.0)
        assertEquals(0.0, si.right, 0.0)
    }

    @Test
    fun `status bar only populates top`() {
        val si = computeInsets(insets(statusBars = Insets.of(0, 24, 0, 0)), 1f)
        assertEquals(24.0, si.top, 0.0)
        assertEquals(0.0, si.bottom, 0.0)
        assertEquals(0.0, si.left, 0.0)
        assertEquals(0.0, si.right, 0.0)
    }

    @Test
    fun `nav bar only populates bottom`() {
        val si = computeInsets(insets(navigationBars = Insets.of(0, 0, 0, 48)), 1f)
        assertEquals(48.0, si.bottom, 0.0)
        assertEquals(0.0, si.top, 0.0)
    }

    @Test
    fun `cutout deeper than status bar wins on top`() {
        val si = computeInsets(
            insets(
                statusBars = Insets.of(0, 24, 0, 0),
                displayCutout = Insets.of(0, 40, 0, 0),
            ),
            1f,
        )
        // top = max(statusBars.top=24, cutout.top=40) = 40
        assertEquals(40.0, si.top, 0.0)
        assertEquals(0.0, si.left, 0.0)
        assertEquals(0.0, si.right, 0.0)
    }

    @Test
    fun `side cutout surfaces on left and right`() {
        // Landscape punch-hole on the side: cutout left=30, right=10.
        val si = computeInsets(insets(displayCutout = Insets.of(30, 0, 10, 0)), 1f)
        assertEquals(30.0, si.left, 0.0)
        assertEquals(10.0, si.right, 0.0)
        assertEquals(0.0, si.top, 0.0)
    }

    @Test
    fun `density scales raw px to css px`() {
        val si = computeInsets(insets(statusBars = Insets.of(0, 48, 0, 0)), 2f)
        assertEquals(24.0, si.top, 0.001)
    }
}
