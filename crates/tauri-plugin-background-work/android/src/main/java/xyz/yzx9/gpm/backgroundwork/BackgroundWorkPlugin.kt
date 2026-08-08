// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package xyz.yzx9.gpm.backgroundwork

import android.app.Activity
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin

@InvokeArg
class ScheduleArgs {
    var intervalHours: Long = 0
    var configDir: String? = null
    var workerClassName: String? = null
    var workName: String? = null
}

@InvokeArg
class CancelArgs {
    var workName: String? = null
}

/**
 * The Tauri-facing plugin: Rust (the `set_background_sync` command + the app
 * setup hook) calls `schedule`/`cancel` over the plugin IPC to drive the
 * WorkManager periodic work. Worker- and name-agnostic — the caller passes the
 * worker class FQN and the unique-work name, so this plugin carries no
 * gpm-specific identifier. The frontend never invokes this directly.
 */
@TauriPlugin
class BackgroundWorkPlugin(private val activity: Activity) : Plugin(activity) {
    @Command
    fun schedule(invoke: Invoke) {
        val args = invoke.parseArgs(ScheduleArgs::class.java)
        val configDir = args.configDir
        val workerClassName = args.workerClassName
        val workName = args.workName
        if (configDir.isNullOrEmpty() || workerClassName.isNullOrEmpty() || workName.isNullOrEmpty()) {
            invoke.reject("missing configDir/workerClassName/workName", "BG_WORK_ARGS")
            return
        }
        try {
            BackgroundWorkScheduler.schedule(
                activity.applicationContext,
                args.intervalHours,
                configDir,
                workerClassName,
                workName,
            )
        } catch (e: ClassNotFoundException) {
            // A wrong/stale worker FQN must surface, not silently no-op.
            invoke.reject("worker class not found: $workerClassName", "BG_WORKER_CLASS")
            return
        } catch (e: ClassCastException) {
            // The FQN resolved but isn't a ListenableWorker subclass.
            invoke.reject("$workerClassName is not a ListenableWorker", "BG_WORKER_CLASS")
            return
        } catch (e: ExceptionInInitializerError) {
            // The worker's static init failed — e.g. System.loadLibrary in
            // SyncWorker's companion threw UnsatisfiedLinkError (an Error, not
            // caught by ClassNotFoundException). Surface the real cause so it
            // isn't swallowed by Rust's `let _ =` as a generic Tauri error.
            val c = e.cause ?: e
            invoke.reject(
                "worker class init failed: ${c.javaClass.simpleName}: ${c.message}",
                "BG_WORKER_CLASS",
            )
            return
        }
        invoke.resolve()
    }

    @Command
    fun cancel(invoke: Invoke) {
        val args = invoke.parseArgs(CancelArgs::class.java)
        val workName = args.workName
        if (workName.isNullOrEmpty()) {
            invoke.reject("missing workName", "BG_WORK_ARGS")
            return
        }
        BackgroundWorkScheduler.cancel(activity.applicationContext, workName)
        invoke.resolve()
    }
}
