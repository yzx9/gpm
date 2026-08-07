// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package xyz.yzx9.gpm.filesave

import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.InputStream
import org.junit.Assert.assertArrayEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Characterization tests for [streamCopy] — the staged-file → destination
 * streaming loop, at the empty / boundary / exact-boundary sizes and with
 * partial reads that ByteArrayInputStream never produces. A regression to
 * `write(buffer, 0, buffer.size)` (ignoring the read length) would corrupt the
 * bundle on the way to the chosen destination.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class FileSavePluginTest {

    @Test
    fun streamCopy_emptySource() {
        val dst = ByteArrayOutputStream()
        streamCopy(ByteArrayInputStream(ByteArray(0)), dst)
        assertArrayEquals(ByteArray(0), dst.toByteArray())
    }

    @Test
    fun streamCopy_singleByte() {
        val dst = ByteArrayOutputStream()
        streamCopy(ByteArrayInputStream(byteArrayOf(0x41)), dst)
        assertArrayEquals(byteArrayOf(0x41), dst.toByteArray())
    }

    @Test
    fun streamCopy_exactBoundary() {
        // The buffer is 0xFFFF (65535); a source of exactly that size is the boundary.
        val input = ByteArray(0xFFFF) { (it % 251).toByte() }
        val dst = ByteArrayOutputStream()
        streamCopy(ByteArrayInputStream(input), dst)
        assertArrayEquals(input, dst.toByteArray())
    }

    @Test
    fun streamCopy_boundaryPlusOne() {
        // One past the boundary forces a second read (full buffer + 1 byte).
        val input = ByteArray(0x10000) { (it % 251).toByte() }
        val dst = ByteArrayOutputStream()
        streamCopy(ByteArrayInputStream(input), dst)
        assertArrayEquals(input, dst.toByteArray())
    }

    @Test
    fun streamCopy_twiceBoundary() {
        val input = ByteArray(0x1FFFE) { (it % 251).toByte() }
        val dst = ByteArrayOutputStream()
        streamCopy(ByteArrayInputStream(input), dst)
        assertArrayEquals(input, dst.toByteArray())
    }

    @Test
    fun streamCopy_partialReadsRoundTrip() {
        // A real ContentResolver stream can return short reads mid-stream.
        // ByteArrayInputStream always fills the buffer, so use a chunked stream
        // (≤7 bytes/read) to prove the loop honors the actual read length.
        val input = ByteArray(0x10000) { (it % 251).toByte() }
        val dst = ByteArrayOutputStream()
        streamCopy(ChunkedStream(input, 7), dst)
        assertArrayEquals(input, dst.toByteArray())
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
