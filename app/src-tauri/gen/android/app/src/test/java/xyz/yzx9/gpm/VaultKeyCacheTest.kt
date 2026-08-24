// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package xyz.yzx9.gpm

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** The process-lifetime cache state machine: empty → set → get → clear. */
class VaultKeyCacheTest {

    @Test
    fun cachesAndClears() {
        VaultKeyCache.clear()
        assertNull(VaultKeyCache.get())
        VaultKeyCache.set("key-b64")
        assertEquals("key-b64", VaultKeyCache.get())
        VaultKeyCache.clear()
        assertNull(VaultKeyCache.get())
    }
}
