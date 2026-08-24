// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package xyz.yzx9.gpm

import org.json.JSONObject

/** One fillable entry as the Rust list core reports it. */
data class FillEntry(val repoId: String, val name: String) {
    override fun toString(): String = name
}

/** Typed view of the Rust `FillListResult` JSON (`AutofillJson.parseList`). */
sealed class FillListOutcome {
    data class Ok(val entries: List<FillEntry>) : FillListOutcome()

    data class Skipped(val reason: String) : FillListOutcome()

    data class Error(val message: String) : FillListOutcome()
}

/** Typed view of the Rust `FillDecryptResult` JSON (`AutofillJson.parseDecrypt`). */
sealed class FillDecryptOutcome {
    data class Ok(val password: String, val username: String) : FillDecryptOutcome()

    data class Skipped(val reason: String) : FillDecryptOutcome()

    data class Error(val message: String) : FillDecryptOutcome()
}

/**
 * Headless autofill JNI bridge (R056) — the fill-surface mirror of
 * [SyncWorker]'s externals: loads `libgpm_lib.so` and crosses into the
 * `jni_fill` cores, which return the result enums as JSON. Parsed into typed
 * outcomes by [AutofillJson]; the JSON keys are pinned on the Rust side
 * (`jni_fill::tests::wire_pins_exact_json_keys`) and here.
 */
object AutofillBridge {
    init {
        System.loadLibrary("gpm_lib")
    }

    @JvmStatic
    external fun nativeListEntries(configDir: String, masterKeyB64: String): String

    @JvmStatic
    external fun nativeDecryptEntry(
        configDir: String,
        masterKeyB64: String,
        vaultKeyB64: String,
        repoId: String,
        entryName: String,
    ): String
}

/** Parses the bridge JSON into typed outcomes (exact serde keys, no guessing). */
object AutofillJson {
    fun parseList(json: String): FillListOutcome = when (val status = JSONObject(json).optString("status")) {
        "ok" -> FillListOutcome.Ok(
            JSONObject(json).optJSONArray("entries")?.let { arr ->
                (0 until arr.length()).map { i ->
                    val e = arr.getJSONObject(i)
                    FillEntry(e.getString("repo_id"), e.getString("name"))
                }
            } ?: emptyList(),
        )
        "skipped" -> FillListOutcome.Skipped(JSONObject(json).getString("reason"))
        else -> FillListOutcome.Error("unexpected list status: $status")
    }

    fun parseDecrypt(json: String): FillDecryptOutcome {
        val obj = JSONObject(json)
        return when (val status = obj.optString("status")) {
            "ok" -> FillDecryptOutcome.Ok(
                password = obj.getString("password"),
                username = obj.getString("username"),
            )
            "skipped" -> FillDecryptOutcome.Skipped(obj.getString("reason"))
            else -> FillDecryptOutcome.Error("unexpected decrypt status: $status")
        }
    }
}
