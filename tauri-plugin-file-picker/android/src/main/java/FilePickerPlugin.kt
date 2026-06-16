// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0
//
// Backend-only native file picker for gpm: opens the Android Storage Access
// Framework picker (ACTION_OPEN_DOCUMENT) and reads the picked file's bytes via
// the ContentResolver, returning them base64-encoded to Rust. The file contents
// flow Kotlin → Rust and never reach the WebView.
//
// The picker pattern (startActivityForResult + @ActivityCallback) mirrors the
// official tauri-plugin-dialog's DialogPlugin.showFilePicker; the byte read
// (ContentResolver → ByteArrayOutputStream → Base64) mirrors its
// FilePickerUtils.getDataFromUri. Unlike dialog, this plugin returns the *bytes*
// rather than just the content:// URI, so Rust can read identity files without
// the fs plugin's content:// handling.

package xyz.yzx9.gpm.filepicker

import android.app.Activity
import android.content.Intent
import android.net.Uri
import android.provider.OpenableColumns
import android.util.Base64
import androidx.activity.result.ActivityResult
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.io.InputStream

/**
 * Backend-only SAF file picker that reads the picked file's bytes into Rust.
 *
 * Registered from Rust via `register_android_plugin("xyz.yzx9.gpm.filepicker",
 * "FilePickerPlugin")` and invoked through the `tauri-plugin-file-picker` handle.
 */
@TauriPlugin
class FilePickerPlugin(private val activity: Activity) : Plugin(activity) {

    /** Open the SAF picker for any openable document. */
    @Command
    fun pick(invoke: Invoke) {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "*/*"
        }
        startActivityForResult(invoke, intent, "onPickResult")
    }

    /** Resolve the pick: read the bytes + display name, base64-encode to Rust. */
    @ActivityCallback
    fun onPickResult(invoke: Invoke, result: ActivityResult) {
        if (result.resultCode != Activity.RESULT_OK) {
            invoke.reject("File picker cancelled", "CANCELLED")
            return
        }
        val uri: Uri = result.data?.data ?: run {
            invoke.reject("No file selected", "CANCELLED")
            return
        }

        val bytes = try {
            readBytes(uri)
        } catch (e: IOException) {
            invoke.reject(safeName(e), "IO_ERROR")
            return
        } ?: run {
            invoke.reject("Failed to open file", "IO_ERROR")
            return
        }

        try {
            val ret = JSObject()
            ret.put("bytes_b64", Base64.encodeToString(bytes, Base64.NO_WRAP))
            ret.put("filename", displayName(uri))
            invoke.resolve(ret)
        } finally {
            // Best-effort wipe of the in-memory copy. A JVM String for the
            // base64 is unavoidable at the resolve hop back to Rust and lives
            // only briefly (matches the keystore plugin's posture).
            bytes.fill(0)
        }
    }

    // ── helpers ───────────────────────────────────────────────────────────

    /** Read the entire content stream into a byte array, or null on open failure. */
    @Throws(IOException::class)
    private fun readBytes(uri: Uri): ByteArray? {
        val stream: InputStream = activity.contentResolver.openInputStream(uri) ?: return null
        stream.use {
            val out = ByteArrayOutputStream()
            val buffer = ByteArray(0xFFFF)
            var len = it.read(buffer)
            while (len != -1) {
                out.write(buffer, 0, len)
                len = it.read(buffer)
            }
            return out.toByteArray()
        }
    }

    /** Best-effort display name from OpenableColumns, falling back to the URI tail. */
    private fun displayName(uri: Uri): String? {
        activity.contentResolver
            .query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
            ?.use { cursor ->
                if (cursor.moveToFirst()) {
                    val idx = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                    if (idx >= 0) {
                        return cursor.getString(idx)
                    }
                }
            }
        return uri.lastPathSegment
    }

    /** Class name only — never leak file contents or provider internals. */
    private fun safeName(e: Throwable): String = e.javaClass.simpleName.ifEmpty { "error" }
}
