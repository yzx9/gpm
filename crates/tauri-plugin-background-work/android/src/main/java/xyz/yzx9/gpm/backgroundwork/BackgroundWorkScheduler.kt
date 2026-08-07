// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package xyz.yzx9.gpm.backgroundwork

import android.content.Context
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ListenableWorker
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequest
import androidx.work.WorkManager
import androidx.work.workDataOf
import java.util.concurrent.TimeUnit

/**
 * Enqueues / cancels a periodic, **worker-agnostic** WorkManager job. The
 * caller supplies the worker class FQN (`workerClassName`) and the unique-work
 * name (`workName`); this object carries no app-specific identifier. The worker
 * class is resolved via reflection ([Class.forName] + [asSubclass]), so the
 * plugin depends on no concrete worker.
 *
 * `enqueueUniquePeriodicWork` with `REPLACE` makes a cadence change take
 * effect immediately and keeps the work unique (no duplicate schedules). The
 * `configDir` is forwarded as `InputData` so the Worker reads it from there
 * and never reconstructs the path (Rust is the single source of truth).
 * WorkManager persists periodic work across reboots, so no `BOOT_COMPLETED`
 * receiver is needed. A wrong/stale `workerClassName` throws
 * [ClassNotFoundException], surfaced by the plugin as `BG_WORKER_CLASS`.
 */
object BackgroundWorkScheduler {
    const val KEY_CONFIG_DIR = "config_dir"

    fun schedule(
        context: Context,
        intervalHours: Long,
        configDir: String,
        workerClassName: String,
        workName: String,
    ) {
        val workerClass = Class.forName(workerClassName)
            .asSubclass(ListenableWorker::class.java)
        val constraints =
            Constraints.Builder()
                .setRequiredNetworkType(NetworkType.CONNECTED)
                .build()
        val request =
            PeriodicWorkRequest.Builder(workerClass, intervalHours, TimeUnit.HOURS)
                .setConstraints(constraints)
                .setInputData(workDataOf(KEY_CONFIG_DIR to configDir))
                .build()
        WorkManager.getInstance(context)
            .enqueueUniquePeriodicWork(workName, ExistingPeriodicWorkPolicy.REPLACE, request)
    }

    fun cancel(context: Context, workName: String) {
        WorkManager.getInstance(context).cancelUniqueWork(workName)
    }
}
