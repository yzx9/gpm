// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

package xyz.yzx9.gpm.securekeystore

import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Characterization tests for [SecureKeystorePlugin]'s pure helpers, including the
 * auth-free store's Base64 round-trip (`encodeBlob`/`decodeBlob`).
 *
 * These lock the plugin's *current* behavior; they do NOT detect cross-plugin
 * drift with biometric-keystore's copies (a unilateral change passes both
 * suites). Drift detection needs the deferred shared-module de-dup (RFC-0041).
 * `decodeBlob` preserves the original `readCipherData` semantics exactly: null
 * iff an input is null (nothing sealed); a present-but-empty string decodes to
 * an empty `ByteArray` (NOT null) — characterization, not a behavior change.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class SecureKeystorePluginTest {

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
        assertEquals("BIOMETRIC_FAILED", mapErrorCode(BiometricPrompt.ERROR_UNABLE_TO_PROCESS))
        assertEquals("BIOMETRIC_FAILED", mapErrorCode(BiometricPrompt.ERROR_NO_SPACE))
        assertEquals("BIOMETRIC_FAILED", mapErrorCode(BiometricPrompt.ERROR_TIMEOUT))
        assertEquals("BIOMETRIC_FAILED", mapErrorCode(99999))
    }

    @Test
    fun safeName_returnsSimpleClassName() {
        assertEquals("IllegalStateException", safeName(IllegalStateException("x")))
    }

    @Test
    fun safeName_fallsBackWhenSimpleNameEmpty() {
        val anon = object : Throwable() {}
        assertEquals("error", safeName(anon))
    }

    @Test
    fun encodeBlob_decodeBlob_roundTripsBytes() {
        val iv = byteArrayOf(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12)
        val ct = byteArrayOf(0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80.toByte())
        val (ivB64, ctB64) = encodeBlob(iv, ct)
        val decoded = decodeBlob(ivB64, ctB64)
        assertEquals(iv.toList(), decoded!!.first.toList())
        assertEquals(ct.toList(), decoded.second.toList())
    }

    @Test
    fun decodeBlob_nullWhenEitherInputNull() {
        // Presence is folded into null: an absent pref (null) ⇒ nothing sealed.
        assertNull(decodeBlob(null, "x"))
        assertNull(decodeBlob("x", null))
        assertNull(decodeBlob(null, null))
    }

    @Test
    fun decodeBlob_presentEmptyStringDecodesToEmptyArray() {
        // Preserves the original readCipherData semantics: a present-but-empty
        // pref decodes (to empty), NOT null. Only an absent pref yields null.
        val decoded = decodeBlob("", "")!!
        assertEquals(0, decoded.first.size)
        assertEquals(0, decoded.second.size)
    }

    // mapBiometricState — byte-identical to biometric-keystore's copy (the type
    // is duplicated across the two plugin crates; this catches drift). The
    // BIOMETRIC_* constants are compile-time inlined.
    @Test
    fun mapBiometricState_success_returns_available() {
        assertEquals(
            "available",
            mapBiometricState(
                BiometricManager.BIOMETRIC_SUCCESS,
                BiometricManager.BIOMETRIC_SUCCESS,
            ),
        )
    }

    @Test
    fun mapBiometricState_noneEnrolledWithWeakPrint_returns_weakEnrolled() {
        assertEquals(
            "weak_enrolled",
            mapBiometricState(
                BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED,
                BiometricManager.BIOMETRIC_SUCCESS,
            ),
        )
    }

    @Test
    fun mapBiometricState_noneEnrolledNothingEnrolled_returns_noEnrollment() {
        assertEquals(
            "no_enrollment",
            mapBiometricState(
                BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED,
                BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED,
            ),
        )
    }

    @Test
    fun mapBiometricState_noHardware_returns_unavailable() {
        assertEquals(
            "unavailable",
            mapBiometricState(
                BiometricManager.BIOMETRIC_ERROR_NO_HARDWARE,
                BiometricManager.BIOMETRIC_ERROR_NO_HARDWARE,
            ),
        )
    }

    @Test
    fun mapBiometricState_hwUnavailable_returns_unavailable() {
        assertEquals(
            "unavailable",
            mapBiometricState(
                BiometricManager.BIOMETRIC_ERROR_HW_UNAVAILABLE,
                BiometricManager.BIOMETRIC_ERROR_HW_UNAVAILABLE,
            ),
        )
    }
}
