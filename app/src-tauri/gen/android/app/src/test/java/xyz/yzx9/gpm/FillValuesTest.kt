// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package xyz.yzx9.gpm

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * Pins the decrypt-result → focused-field mapping: the password target
 * always fills, the username target only on a non-empty username, and a
 * password-only screen gets only the password.
 */
class FillValuesTest {

    @Test
    fun mapsBothFields() {
        val targets = FillTargets("u", "p")
        assertEquals(
            mapOf("u" to "alice", "p" to "s3cret"),
            FillContract.mapFillValues(targets, "alice", "s3cret"),
        )
    }

    @Test
    fun passwordOnlyTarget() {
        val targets = FillTargets(null, "p")
        assertEquals(
            mapOf("p" to "s3cret"),
            FillContract.mapFillValues(targets, "alice", "s3cret"),
        )
    }

    @Test
    fun emptyUsernameIsOmitted() {
        val targets = FillTargets("u", "p")
        assertEquals(
            mapOf("p" to "s3cret"),
            FillContract.mapFillValues(targets, "", "s3cret"),
        )
    }

    @Test
    fun nullUsernameIsOmitted() {
        val targets = FillTargets("u", "p")
        assertEquals(
            mapOf("p" to "s3cret"),
            FillContract.mapFillValues(targets, null, "s3cret"),
        )
    }
}
