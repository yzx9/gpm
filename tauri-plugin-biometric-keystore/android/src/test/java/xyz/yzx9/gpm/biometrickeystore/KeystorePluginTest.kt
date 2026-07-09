// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

package xyz.yzx9.gpm.biometrickeystore

import androidx.biometric.BiometricPrompt
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Characterization tests for [KeystorePlugin]'s pure helpers.
 *
 * These lock the plugin's *current* behavior. They do NOT detect cross-plugin
 * drift with secure-keystore's copies — a unilateral change to one plugin's
 * mapping passes both suites. Drift detection needs the deferred shared-module
 * de-dup (RFC-0041); the exhaustive `mapErrorCode` table here makes a divergence
 * visible at review time and doubles as the safety net for that future refactor.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class KeystorePluginTest {

    @Test
    fun resolvePromptText_appliesGenericFallbacks() {
        val r = resolvePromptText(null, "  ", null)
        assertEquals("gpm", r.title)
        assertEquals(null, r.subtitle)
        assertEquals("Cancel", r.negative)
    }

    @Test
    fun resolvePromptText_keepsProvidedText() {
        val r = resolvePromptText("Title", "Sub", "Neg")
        assertEquals("Title", r.title)
        assertEquals("Sub", r.subtitle)
        assertEquals("Neg", r.negative)
    }

    @Test
    fun resolvePromptText_dropsBlankSubtitleOnly() {
        val r = resolvePromptText("", "", "")
        assertEquals("gpm", r.title)
        assertEquals(null, r.subtitle)
        assertEquals("Cancel", r.negative)
    }

    @Test
    fun mapErrorCode_cancellations() {
        assertEquals("BIOMETRIC_CANCELLED", mapErrorCode(BiometricPrompt.ERROR_USER_CANCELED))
        assertEquals("BIOMETRIC_CANCELLED", mapErrorCode(BiometricPrompt.ERROR_NEGATIVE_BUTTON))
        assertEquals("BIOMETRIC_CANCELLED", mapErrorCode(BiometricPrompt.ERROR_CANCELED))
    }

    @Test
    fun mapErrorCode_unavailable() {
        assertEquals("BIOMETRIC_UNAVAILABLE", mapErrorCode(BiometricPrompt.ERROR_HW_NOT_PRESENT))
        assertEquals("BIOMETRIC_UNAVAILABLE", mapErrorCode(BiometricPrompt.ERROR_HW_UNAVAILABLE))
        assertEquals("BIOMETRIC_UNAVAILABLE", mapErrorCode(BiometricPrompt.ERROR_NO_BIOMETRICS))
        assertEquals("BIOMETRIC_UNAVAILABLE", mapErrorCode(BiometricPrompt.ERROR_NO_DEVICE_CREDENTIAL))
        assertEquals("BIOMETRIC_UNAVAILABLE", mapErrorCode(BiometricPrompt.ERROR_SECURITY_UPDATE_REQUIRED))
    }

    @Test
    fun mapErrorCode_lockout() {
        assertEquals("BIOMETRIC_LOCKOUT", mapErrorCode(BiometricPrompt.ERROR_LOCKOUT))
        assertEquals("BIOMETRIC_LOCKOUT", mapErrorCode(BiometricPrompt.ERROR_LOCKOUT_PERMANENT))
    }

    @Test
    fun mapErrorCode_unknownCodesCollapseToFailed() {
        // Framework codes not handled above collapse to the default bucket.
        assertEquals("BIOMETRIC_FAILED", mapErrorCode(BiometricPrompt.ERROR_UNABLE_TO_PROCESS))
        assertEquals("BIOMETRIC_FAILED", mapErrorCode(BiometricPrompt.ERROR_NO_SPACE))
        assertEquals("BIOMETRIC_FAILED", mapErrorCode(BiometricPrompt.ERROR_TIMEOUT))
        // An entirely unknown code also collapses.
        assertEquals("BIOMETRIC_FAILED", mapErrorCode(99999))
    }

    @Test
    fun safeName_returnsSimpleClassName() {
        assertEquals("IllegalStateException", safeName(IllegalStateException("x")))
    }

    @Test
    fun safeName_fallsBackWhenSimpleNameEmpty() {
        // An anonymous throwable subclass has an empty simple name.
        val anon = object : Throwable() {}
        assertEquals("error", safeName(anon))
    }
}
