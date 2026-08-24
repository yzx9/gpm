// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package xyz.yzx9.gpm

import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Pins the Rust↔Kotlin JSON contract for the fill bridge (Robolectric
 * provides `org.json`): the exact serde keys the Rust cores emit — the same
 * IPC-drift defense as the plugin contract tests.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class AutofillJsonTest {

    @Test
    fun parsesListOkWithEntries() {
        val outcome =
            AutofillJson.parseList(
                """{"status":"ok","entries":[
                   {"repo_id":"ab","name":"cloud/aws/root"},
                   {"repo_id":"cd","name":"web/example/alice"}]}""",
            )
        assertEquals(
            listOf(
                FillEntry("ab", "cloud/aws/root"),
                FillEntry("cd", "web/example/alice"),
            ),
            (outcome as FillListOutcome.Ok).entries,
        )
    }

    @Test
    fun parsesListOkWithEmptyEntries() {
        val outcome = AutofillJson.parseList("""{"status":"ok","entries":[]}""")
        assertEquals(emptyList<FillEntry>(), (outcome as FillListOutcome.Ok).entries)
    }

    @Test
    fun parsesListSkippedReason() {
        val outcome = AutofillJson.parseList("""{"status":"skipped","reason":"no_key"}""")
        assertEquals("no_key", (outcome as FillListOutcome.Skipped).reason)
    }

    @Test
    fun parsesDecryptOk() {
        val outcome =
            AutofillJson.parseDecrypt("""{"status":"ok","password":"p","username":"u"}""")
        val ok = outcome as FillDecryptOutcome.Ok
        assertEquals("p", ok.password)
        assertEquals("u", ok.username)
    }

    @Test
    fun parsesDecryptSkippedReason() {
        val outcome = AutofillJson.parseDecrypt("""{"status":"skipped","reason":"app_locked"}""")
        assertEquals("app_locked", (outcome as FillDecryptOutcome.Skipped).reason)
    }

    @Test
    fun unknownStatusIsError() {
        // Defensive: a Rust-side enum rename must surface as Error, never a
        // silent misparse.
        assertEquals(
            FillListOutcome.Error::class.java,
            AutofillJson.parseList("""{"status":"weird"}""").javaClass,
        )
        assertEquals(
            FillDecryptOutcome.Error::class.java,
            AutofillJson.parseDecrypt("""{"status":"weird"}""").javaClass,
        )
    }
}
