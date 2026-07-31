// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

package xyz.yzx9.gpm.backgroundsync

import android.content.Context
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.workDataOf
import java.util.concurrent.TimeUnit

/**
 * Enqueues / cancels the periodic background-sync [SyncWorker] via WorkManager.
 *
 * `enqueueUniquePeriodicWork` with `REPLACE` makes a cadence change take effect
 * immediately and keeps the work unique (no duplicate schedules). The
 * `config_dir` is forwarded as `InputData` so the Worker reads it from there
 * and never reconstructs the path (Rust is the single source of truth — D2).
 * WorkManager persists periodic work across reboots, so no `BOOT_COMPLETED`
 * receiver is needed.
 */
object BackgroundSyncScheduler {
    const val WORK_NAME = "gpm_background_sync"
    const val KEY_CONFIG_DIR = "config_dir"

    fun schedule(context: Context, intervalHours: Long, configDir: String) {
        val constraints =
            Constraints.Builder()
                .setRequiredNetworkType(NetworkType.CONNECTED)
                .build()
        val request =
            PeriodicWorkRequestBuilder<SyncWorker>(intervalHours, TimeUnit.HOURS)
                .setConstraints(constraints)
                .setInputData(workDataOf(KEY_CONFIG_DIR to configDir))
                .build()
        WorkManager.getInstance(context)
            .enqueueUniquePeriodicWork(WORK_NAME, ExistingPeriodicWorkPolicy.REPLACE, request)
    }

    fun cancel(context: Context) {
        WorkManager.getInstance(context).cancelUniqueWork(WORK_NAME)
    }
}
