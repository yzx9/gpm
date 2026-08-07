// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package xyz.yzx9.gpm.keystore

import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import app.tauri.plugin.Invoke
import com.fasterxml.jackson.databind.DeserializationFeature
import com.fasterxml.jackson.databind.ObjectMapper
import java.lang.reflect.Modifier
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Characterization tests for [KeystorePlugin]'s pure helpers, including the
 * auth-free store's Base64 round-trip (`encodeBlob`/`decodeBlob`) and the
 * `openSecuritySettings` deep-link wiring.
 *
 * These lock the plugin's *current* behavior. `decodeBlob` preserves the
 * original `readCipherData` semantics exactly: null iff an input is null
 * (nothing sealed); a present-but-empty string decodes to an empty
 * `ByteArray` (NOT null) — characterization, not a behavior change.
 *
 * NOTE: `resolvePromptText` (brand fallbacks), `BiometricSlot.fromString`, and
 * `hasUsableBiometricInAnySlot` (the app-lock OR) live in the app layer (the
 * plugin carries no brand string, no slot enum, no app-lock logic) — their tests
 * live there. The plugin's pure helpers stay here.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class KeystorePluginTest {

    @Test
    fun mapErrorCode_cancellations() {
        assertEquals("KEYSTORE_CANCELLED", mapErrorCode(BiometricPrompt.ERROR_USER_CANCELED))
        assertEquals("KEYSTORE_CANCELLED", mapErrorCode(BiometricPrompt.ERROR_NEGATIVE_BUTTON))
        assertEquals("KEYSTORE_CANCELLED", mapErrorCode(BiometricPrompt.ERROR_CANCELED))
    }

    @Test
    fun mapErrorCode_unavailable() {
        assertEquals("KEYSTORE_UNAVAILABLE", mapErrorCode(BiometricPrompt.ERROR_HW_NOT_PRESENT))
        assertEquals("KEYSTORE_UNAVAILABLE", mapErrorCode(BiometricPrompt.ERROR_HW_UNAVAILABLE))
        assertEquals("KEYSTORE_UNAVAILABLE", mapErrorCode(BiometricPrompt.ERROR_NO_BIOMETRICS))
        assertEquals("KEYSTORE_UNAVAILABLE", mapErrorCode(BiometricPrompt.ERROR_NO_DEVICE_CREDENTIAL))
        assertEquals("KEYSTORE_UNAVAILABLE", mapErrorCode(BiometricPrompt.ERROR_SECURITY_UPDATE_REQUIRED))
    }

    @Test
    fun mapErrorCode_lockout() {
        assertEquals("KEYSTORE_LOCKOUT", mapErrorCode(BiometricPrompt.ERROR_LOCKOUT))
        assertEquals("KEYSTORE_LOCKOUT", mapErrorCode(BiometricPrompt.ERROR_LOCKOUT_PERMANENT))
    }

    @Test
    fun mapErrorCode_unknownCodesCollapseToFailed() {
        // Framework codes not handled above collapse to the default bucket.
        assertEquals("KEYSTORE_FAILED", mapErrorCode(BiometricPrompt.ERROR_UNABLE_TO_PROCESS))
        assertEquals("KEYSTORE_FAILED", mapErrorCode(BiometricPrompt.ERROR_NO_SPACE))
        assertEquals("KEYSTORE_FAILED", mapErrorCode(BiometricPrompt.ERROR_TIMEOUT))
        // An entirely unknown code also collapses.
        assertEquals("KEYSTORE_FAILED", mapErrorCode(99999))
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

    // ── value byte-flow (encodeValue/decodeValue) ────────────────────────
    //
    // The regression guard for the v0.17.0→v0.17.1 fix: the plugin must seal the
    // RAW bytes the caller passes (Base64 over IPC, decoded before encrypt), NOT
    // their UTF-8. `decodeValue`/`encodeValue` are the inverse pair over
    // `Base64.NO_WRAP`, matching the Rust plugin crate's STANDARD engine.

    @Test
    fun decodeValue_encodeValue_roundTripsArbitraryBytes() {
        // Arbitrary bytes (incl. non-UTF-8: 0x80, 0xff, 0x00) round-trip — the
        // 32 raw key bytes stored by the v0.17.0 on-disk format are such bytes,
        // so this proves the value flow never String-ifies the key.
        val raw = byteArrayOf(0, 1, 2, 0x7f, 0x80.toByte(), 0xff.toByte(), 0x00, 0x42)
        assertEquals(raw.toList(), decodeValue(encodeValue(raw)).toList())
    }

    @Test
    fun encodeValue_isNoWrapBase64_matchingRustStandard() {
        // A 32-byte key → 44-char one-line base64 (no line wrap), the wire shape
        // the Rust plugin crate's STANDARD engine produces.
        val key = ByteArray(32) { it.toByte() }
        val b64 = encodeValue(key)
        assertEquals(44, b64.length)
        assertFalse(b64.contains("\n"))
        assertEquals(key.toList(), decodeValue(b64).toList())
    }

    // mapBiometricState — exhaustive over the canAuthenticate() returns the page
    // distinguishes. The BIOMETRIC_* constants are compile-time inlined, so
    // these are plain value assertions.
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
        // A weak (Class 2) print is enrolled but no STRONG one; gpm needs Class 3.
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

    @Test
    fun securitySettingsIntent_carriesAction() {
        // Pins the biometric-enrollment deep-link target: the system Security
        // settings action (the clipboard-notify sibling pins its own intent).
        assertEquals(
            android.provider.Settings.ACTION_SECURITY_SETTINGS,
            securitySettingsIntent().action,
        )
    }

    @Test
    fun resolveSecuritySettingsDeepLink_true_when_launch_succeeds() {
        assertEquals(true, resolveSecuritySettingsDeepLink { /* no throw */ })
    }

    @Test
    fun resolveSecuritySettingsDeepLink_false_when_launch_throws() {
        // The no-handler-activity fallback (rare OEM ROM): the `@Command`
        // resolves `{ opened: false }` so the caller toasts instead of failing
        // silently. Driven through the extracted helper so no shadow Activity is
        // needed.
        assertEquals(
            false,
            resolveSecuritySettingsDeepLink { throw android.content.ActivityNotFoundException() },
        )
    }

    @Test
    fun openSecuritySettings_isExposedAsACommand() {
        // Regression guard: `openSecuritySettings` must be a dispatchable
        // command on the plugin. It was previously absent (only an orphaned
        // helper existed), so the "Open security settings" recovery tap was a
        // silent no-op. Asserting the public method exists with the @Command
        // signature here guards that; the live `startActivity` path is verified
        // on-device.
        val method = KeystorePlugin::class.java.getDeclaredMethod(
            "openSecuritySettings",
            Invoke::class.java,
        )
        assertNotNull(method.getAnnotation(app.tauri.annotation.Command::class.java))
        assertTrue(Modifier.isPublic(method.modifiers))
    }

    // ── IPC contract: Rust Payload ↔ Kotlin @InvokeArg ───────────────────
    //
    // Tauri parses `@InvokeArg` via Jackson `ObjectMapper.readValue(json, cls)`
    // with `FAIL_ON_UNKNOWN_PROPERTIES` *disabled* (tauri-api PluginManager.kt).
    // These pin the flattened-policy contract: the Rust `Payload` emits
    // camelCase top-level fields (authRequired, …) — NOT a nested `policy`
    // object — so the Kotlin Args MUST read them flat. A nested
    // `policy: KeyPolicyArgs?` would never bind (no `policy` key in the JSON)
    // and silently default to auth-free, defeating App Lock / biometric unlock
    // (the P0 this suite guards against).

    private val tauriLikeMapper = ObjectMapper()
        .disable(DeserializationFeature.FAIL_ON_UNKNOWN_PROPERTIES)

    @Test
    fun storeArgs_bindsFlattenedPolicyFromRustPayload() {
        // The exact JSON the Rust `Payload` serializes for PASSPHRASE_POLICY.
        val json = """{"value":"secret","alias":"gpm_passphrase","prefs":"gpm_keystore","authRequired":true,"authBiometricStrong":true,"invalidatedByEnrollment":true,"authValiditySeconds":0,"title":"gpm","subtitle":null,"negative":"Cancel"}"""
        val args = tauriLikeMapper.readValue(json, StoreArgs::class.java)
        assertEquals("secret", args.value)
        assertEquals("gpm_passphrase", args.alias)
        assertEquals(true, args.authRequired)
        assertEquals(true, args.authBiometricStrong)
        assertEquals(true, args.invalidatedByEnrollment)
        assertEquals(0L, args.authValiditySeconds)
        assertEquals("gpm", args.title)
    }

    @Test
    fun retrieveArgs_bindsFlattenedPolicyFromRustPayload() {
        // Sibling of the StoreArgs pin above: the Rust retrieve `Payload` emits
        // the same flat camelCase policy fields (no `value` on retrieve). A
        // nested `policy` object would silently default to auth-free here too.
        val json = """{"alias":"gpm_vault_key","prefs":"gpm_secure_keystoreVault","authRequired":true,"authBiometricStrong":true,"invalidatedByEnrollment":false,"authValiditySeconds":0,"title":"gpm","subtitle":null,"negative":"Cancel"}"""
        val args = tauriLikeMapper.readValue(json, RetrieveArgs::class.java)
        assertEquals("gpm_vault_key", args.alias)
        assertEquals(true, args.authRequired)
        assertEquals(true, args.authBiometricStrong)
        assertEquals(false, args.invalidatedByEnrollment)
        assertEquals(0L, args.authValiditySeconds)
    }

    @Test
    fun storeArgs_aNestedPolicyObjectDoesNotBind() {
        // Characterizes the trap: a payload that nests the policy under a
        // `policy` key leaves the flat fields at their auth-free defaults. This
        // is exactly the regression this test exists to prevent.
        val nested = """{"value":"x","alias":"a","prefs":"p","policy":{"authRequired":true}}"""
        val args = tauriLikeMapper.readValue(nested, StoreArgs::class.java)
        assertEquals(false, args.authRequired)
    }
}
