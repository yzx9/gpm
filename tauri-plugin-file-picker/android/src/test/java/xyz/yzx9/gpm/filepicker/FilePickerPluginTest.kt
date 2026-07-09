// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

package xyz.yzx9.gpm.filepicker

import android.database.MatrixCursor
import android.provider.OpenableColumns
import java.io.ByteArrayInputStream
import java.io.InputStream
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Characterization tests for [FilePickerPlugin]'s pure read-path helpers.
 *
 * `readStreamFully` covers the byte-read loop at the empty / boundary /
 * exact-boundary sizes that would corrupt identity-file bytes on their way into
 * Rust if regressed. `resolveDisplayName` covers the cursor walk + URI-tail
 * fallback, preserving the original null-value semantics exactly.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class FilePickerPluginTest {

    // ── readStreamFully ────────────────────────────────────────────────

    @Test
    fun readStreamFully_emptyStream() {
        assertArrayEquals(ByteArray(0), readStreamFully(ByteArrayInputStream(ByteArray(0))))
    }

    @Test
    fun readStreamFully_singleByte() {
        assertArrayEquals(byteArrayOf(0x41), readStreamFully(ByteArrayInputStream(byteArrayOf(0x41))))
    }

    @Test
    fun readStreamFully_exactBoundary() {
        // The buffer is 0xFFFF (65535); a stream of exactly that size is the boundary.
        val input = ByteArray(0xFFFF) { (it % 251).toByte() }
        assertArrayEquals(input, readStreamFully(ByteArrayInputStream(input)))
    }

    @Test
    fun readStreamFully_boundaryPlusOne() {
        // One past the boundary forces a second read (full buffer + 1 byte).
        val input = ByteArray(0x10000) { (it % 251).toByte() }
        assertArrayEquals(input, readStreamFully(ByteArrayInputStream(input)))
    }

    @Test
    fun readStreamFully_twiceBoundary() {
        val input = ByteArray(0x1FFFE) { (it % 251).toByte() }
        assertArrayEquals(input, readStreamFully(ByteArrayInputStream(input)))
    }

    @Test
    fun readStreamFully_partialReadsRoundTrip() {
        // A real ContentResolver stream can return short reads mid-stream, not
        // just at the boundary. ByteArrayInputStream always fills the buffer, so
        // use a chunked stream (≤7 bytes/read) to prove the loop honors the
        // actual read length — a regression to `write(buffer, 0, buffer.size)`
        // would corrupt here.
        val input = ByteArray(0x10000) { (it % 251).toByte() }
        assertArrayEquals(input, readStreamFully(ChunkedStream(input, 7)))
    }

    // ── resolveDisplayName ─────────────────────────────────────────────

    @Test
    fun resolveDisplayName_hasName() {
        val cursor = MatrixCursor(arrayOf(OpenableColumns.DISPLAY_NAME)).apply {
            addRow(arrayOf("vault.age"))
        }
        assertEquals("vault.age", resolveDisplayName(cursor, "fallback.age"))
    }

    @Test
    fun resolveDisplayName_emptyCursorFallsBack() {
        val cursor = MatrixCursor(arrayOf(OpenableColumns.DISPLAY_NAME)) // no rows
        assertEquals("fallback.age", resolveDisplayName(cursor, "fallback.age"))
    }

    @Test
    fun resolveDisplayName_nullCursorFallsBack() {
        assertEquals("fallback.age", resolveDisplayName(null, "fallback.age"))
    }

    @Test
    fun resolveDisplayName_columnAbsentFallsBack() {
        // A cursor whose only column is NOT DISPLAY_NAME ⇒ getColumnIndex returns -1.
        val cursor = MatrixCursor(arrayOf("_id")).apply { addRow(arrayOf(1)) }
        assertEquals("fallback", resolveDisplayName(cursor, "fallback"))
    }

    @Test
    fun resolveDisplayName_presentColumnNullValueReturnsNull() {
        // Characterization: a present DISPLAY_NAME column with a null value returns
        // null (does NOT fall back) — matches the original `displayName` behavior.
        val cursor = MatrixCursor(arrayOf(OpenableColumns.DISPLAY_NAME)).apply {
            addRow(arrayOf(null))
        }
        assertNull(resolveDisplayName(cursor, "fallback"))
    }

    @Test
    fun resolveDisplayName_nullFallbackAndNullCursorReturnsNull() {
        assertNull(resolveDisplayName(null, null))
    }

    /** An InputStream that returns at most `chunk` bytes per read, exercising
     *  partial reads that ByteArrayInputStream never produces. */
    private class ChunkedStream(data: ByteArray, private val chunk: Int) : InputStream() {
        private val src = data
        private var pos = 0
        override fun read(): Int = if (pos >= src.size) -1 else src[pos++].toInt() and 0xFF
        override fun read(b: ByteArray, off: Int, len: Int): Int {
            if (pos >= src.size) return -1
            val n = minOf(len, chunk, src.size - pos)
            System.arraycopy(src, pos, b, off, n)
            pos += n
            return n
        }
    }
}
