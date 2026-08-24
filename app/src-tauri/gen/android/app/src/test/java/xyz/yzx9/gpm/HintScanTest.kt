// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package xyz.yzx9.gpm

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * Pins the decided MVP detection rule: ONLY username/password hints count
 * (no email/username-adjacent heuristics), and the first field carrying
 * each hint wins. Plain JVM — [FieldHints]/[FillTargets] carry no android
 * classes.
 */
class HintScanTest {

    private fun f(id: String, vararg hints: String) = FieldHints(id, hints.toList())

    @Test
    fun findsBothFields() {
        val targets =
            HintScan.classify(listOf(f("u", "username"), f("p", "password")))
        assertEquals("u", targets.usernameField)
        assertEquals("p", targets.passwordField)
    }

    @Test
    fun passwordOnlyLeavesUsernameNull() {
        val targets = HintScan.classify(listOf(f("p", "password")))
        assertNull(targets.usernameField)
        assertEquals("p", targets.passwordField)
    }

    @Test
    fun usernameOnlyLeavesPasswordNull() {
        val targets = HintScan.classify(listOf(f("u", "username")))
        assertEquals("u", targets.usernameField)
        assertNull(targets.passwordField)
    }

    @Test
    fun noHintsYieldsNoTargets() {
        val targets = HintScan.classify(listOf(f("a"), f("b", "emailAddress")))
        assertNull(targets.usernameField)
        assertNull(targets.passwordField)
    }

    @Test
    fun firstMatchWins() {
        val targets =
            HintScan.classify(
                listOf(f("u1", "username"), f("u2", "username"), f("p2", "password"), f("p1", "password")),
            )
        assertEquals("u1", targets.usernameField)
        assertEquals("p2", targets.passwordField)
    }
}
