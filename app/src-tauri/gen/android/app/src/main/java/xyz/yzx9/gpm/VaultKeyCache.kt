// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package xyz.yzx9.gpm

/**
 * The process-lifetime vault-key cache (R056 v0 trade-off): the first pick
 * in a cold fill surface unseals the key via [VaultBootstrap] and parks it
 * here; later picks in the same process skip the prompt. The OS killing the
 * process is the only wipe — no idle TTL, no in-app re-lock signal (both are
 * follow-up-RFC candidates). Test isolation via [clear].
 */
object VaultKeyCache {
    @Volatile
    private var keyB64: String? = null

    fun get(): String? = keyB64

    fun set(keyB64: String) {
        this.keyB64 = keyB64
    }

    fun clear() {
        keyB64 = null
    }
}
