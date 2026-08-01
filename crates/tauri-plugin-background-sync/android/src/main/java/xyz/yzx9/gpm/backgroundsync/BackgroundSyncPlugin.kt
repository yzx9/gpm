// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

package xyz.yzx9.gpm.backgroundsync

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
}

/**
 * The Tauri-facing plugin: Rust (the `set_background_sync` command + the app
 * setup hook) calls `schedule`/`cancel` over the plugin IPC to drive the
 * WorkManager periodic work. The frontend never invokes this directly.
 */
@TauriPlugin
class BackgroundSyncPlugin(private val activity: Activity) : Plugin(activity) {
    @Command
    fun schedule(invoke: Invoke) {
        val args = invoke.parseArgs(ScheduleArgs::class.java)
        val configDir = args.configDir
        if (configDir.isNullOrEmpty()) {
            invoke.reject("missing configDir", "BG_SYNC_ARGS")
            return
        }
        BackgroundSyncScheduler.schedule(activity.applicationContext, args.intervalHours, configDir)
        invoke.resolve()
    }

    @Command
    fun cancel(invoke: Invoke) {
        BackgroundSyncScheduler.cancel(activity.applicationContext)
        invoke.resolve()
    }
}
