// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Backend-only SAF save for gpm: pops the Android Storage Access Framework
// save picker (ACTION_CREATE_DOCUMENT) and streams a staged file into the chosen
// destination via ContentResolver.openOutputStream. Owns the write so it has a
// real error path — the official tauri-plugin-fs content-URI write is
// sync-blocking and panics on a null fd. The staged file's bytes never reach
// the WebView; only its path crosses to Kotlin.

package xyz.yzx9.gpm.filesave

import android.app.Activity
import android.content.Intent
import android.net.Uri
import androidx.activity.result.ActivityResult
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File
import java.io.IOException
import java.io.InputStream
import java.io.OutputStream

/** Stream `src` into `dst` fully, flushing and closing both. Pure (takes
 *  streams rather than resolving a content URI) so it can be fed constructed
 *  inputs. */
@Throws(IOException::class)
internal fun streamCopy(src: InputStream, dst: OutputStream) {
    src.use { input ->
        dst.use { output ->
            val buffer = ByteArray(0xFFFF)
            var len = input.read(buffer)
            while (len != -1) {
                output.write(buffer, 0, len)
                len = input.read(buffer)
            }
            output.flush()
        }
    }
}

/** Arguments for the `save` command, deserialized from the Rust payload. */
@InvokeArg
class SaveArgs {
    /** Suggested file name offered in the save dialog. */
    lateinit var filename: String
    /** Absolute path of the staged file to stream into the destination. */
    lateinit var tempPath: String
    /** MIME type filter for the picker (e.g. `application/zip`,
     *  `application/octet-stream`). */
    lateinit var mimeType: String
}

/**
 * Backend-only SAF save. Registered from Rust via
 * `register_android_plugin("xyz.yzx9.gpm.filesave", "FileSavePlugin")`.
 *
 * Save is single-flight: the diagnostics export is a one-at-a-time user action,
 * so the staged path is carried to the activity-result callback in a field.
 */
@TauriPlugin
class FileSavePlugin(private val activity: Activity) : Plugin(activity) {

    // Staged path carried from `save` to the `onSaveResult` callback.
    private var pendingTempPath: String? = null

    /** Pop the SAF save dialog for a document of the caller's MIME type. */
    @Command
    fun save(invoke: Invoke) {
        val args = invoke.parseArgs(SaveArgs::class.java)
        if (args.mimeType.isBlank()) {
            invoke.reject("Save requires a MIME type", "SAVE_FAILED")
            return
        }
        val intent = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = args.mimeType
            putExtra(Intent.EXTRA_TITLE, args.filename)
        }
        pendingTempPath = args.tempPath
        startActivityForResult(invoke, intent, "onSaveResult")
    }

    /** Stream the staged file into the chosen destination. */
    @ActivityCallback
    fun onSaveResult(invoke: Invoke, result: ActivityResult) {
        val tempPath = pendingTempPath
        pendingTempPath = null

        if (result.resultCode != Activity.RESULT_OK) {
            invoke.reject("Save cancelled", "CANCELLED")
            return
        }
        val uri: Uri = result.data?.data ?: run {
            invoke.reject("No destination selected", "CANCELLED")
            return
        }
        val path = tempPath ?: run {
            invoke.reject("Missing staged file path", "SAVE_FAILED")
            return
        }
        val src = File(path)
        if (!src.exists()) {
            invoke.reject("Staged file vanished", "SAVE_FAILED")
            return
        }

        try {
            val out = activity.contentResolver.openOutputStream(uri) ?: run {
                invoke.reject("Destination not writable", "SAVE_FAILED")
                return
            }
            streamCopy(src.inputStream(), out)
        } catch (e: IOException) {
            invoke.reject(safeName(e), "IO_ERROR")
            return
        } catch (e: Exception) {
            // openOutputStream can throw SecurityException / IllegalArgumentException
            // (revoked URI grant, exotic provider) — non-IO, so a distinct code.
            invoke.reject(safeName(e), "SAVE_FAILED")
            return
        }

        val ret = JSObject()
        ret.put("ok", true)
        invoke.resolve(ret)
    }

    /** Class name only — never leak file contents or provider internals. */
    private fun safeName(e: Throwable): String = e.javaClass.simpleName.ifEmpty { "error" }
}
