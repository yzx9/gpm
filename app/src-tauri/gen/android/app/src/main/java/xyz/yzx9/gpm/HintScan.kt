// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package xyz.yzx9.gpm

/**
 * One view node's autofill-relevant extract — the `(id, hints)` pair the
 * service's `AssistStructure` walk produces. Kept as a plain data class (no
 * `AssistStructure` dependency) so [HintScan] unit-tests on the JVM.
 */
data class FieldHints<T>(val id: T, val hints: List<String>)

/** The fill targets found on a screen; either field may be absent. */
data class FillTargets<T>(val usernameField: T?, val passwordField: T?)

/**
 * The decided MVP detection rule (R056): only fields declaring the username
 * or password autofill hint are fill targets — no heuristics for hint-less
 * fields, no email/username-adjacent hints. First field carrying each hint
 * wins. Generic over the id type so the scan classifies anything the walker
 * extracts (tests use `String`, the service uses `AutofillId`).
 */
object HintScan {
    fun <T> classify(fields: List<FieldHints<T>>): FillTargets<T> {
        var username: T? = null
        var password: T? = null
        for (field in fields) {
            if (username == null && field.hints.contains(FillContract.HINT_USERNAME)) {
                username = field.id
            }
            if (password == null && field.hints.contains(FillContract.HINT_PASSWORD)) {
                password = field.id
            }
        }
        return FillTargets(username, password)
    }
}
