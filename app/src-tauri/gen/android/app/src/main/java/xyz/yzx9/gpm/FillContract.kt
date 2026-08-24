// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package xyz.yzx9.gpm

/**
 * The service↔activity contract for autofill (R056): the intent extras the
 * [GpmAutofillService] attaches to the fill activity's auth intent, and the
 * gopass-semantics value mapping the activity applies to the decrypt result.
 */
object FillContract {
    /** Focused-field [android.view.autofill.AutofillId]s, in hint order. */
    const val EXTRA_AUTOFILL_IDS = "gpm_autofill_ids"

    /**
     * Same-order hint tags ([HINT_USERNAME]/[HINT_PASSWORD]) for the ids in
     * [EXTRA_AUTOFILL_IDS] — an `AutofillId` carries no hint of its own, so
     * the parallel list is how the activity knows which id is which. The
     * extras are always set (even as empty lists): Android 12+ crashes the
     * fill flow on a null-extras auth intent.
     */
    const val EXTRA_AUTOFILL_HINTS = "gpm_autofill_hints"

    /** Value of `View.AUTOFILL_HINT_USERNAME` (string constants inline). */
    const val HINT_USERNAME = "username"

    /** Value of `View.AUTOFILL_HINT_PASSWORD`. */
    const val HINT_PASSWORD = "password"

    /**
     * Map decrypt values onto the focused-field hints: the password target
     * always gets the password; the username target only when the username
     * is non-empty (the Rust core's path fallback usually fills it).
     * Generic over the field id so tests can drive it with plain strings.
     */
    fun <T> mapFillValues(
        targets: FillTargets<T>,
        username: String?,
        password: String,
    ): Map<T, String> =
        buildMap {
            targets.passwordField?.let { put(it, password) }
            val user = username ?: return@buildMap
            if (user.isNotEmpty()) targets.usernameField?.let { put(it, user) }
        }
}
