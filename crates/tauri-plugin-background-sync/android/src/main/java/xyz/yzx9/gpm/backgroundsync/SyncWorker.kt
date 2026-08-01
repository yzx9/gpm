// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

package xyz.yzx9.gpm.backgroundsync

import android.app.ActivityManager
import android.content.Context
import androidx.work.CoroutineWorker
import androidx.work.WorkerParameters
import org.json.JSONObject

/**
 * The periodic background-sync worker. Runs pull-only (the heavy-autofill
 * persona is read-only), gated on AutoSync + cadence + AppLock (all re-checked
 * in the Rust entry). Loads `libgpm_lib.so` (already packaged by Tauri's
 * `RustPlugin.kt`) and crosses into Rust via [nativeSync].
 *
 * Default no-arg constructor — WorkManager instantiates workers via reflection,
 * including after process death.
 */
class SyncWorker(appContext: Context, params: WorkerParameters) :
    CoroutineWorker(appContext, params) {

    companion object {
        init {
            System.loadLibrary("gpm_lib")
        }

        @JvmStatic
        external fun nativeSync(configDir: String, masterKeyB64: String): String

        private const val MAX_ATTEMPTS = 3
    }

    override suspend fun doWork(): Result {
        // Skip-if-foreground: the app is open — the foreground sync owns
        // convergence, and this avoids contending the cross-process repo lock.
        // Best-effort heuristic (the flock is the real mutex); if this read is
        // unreliable the worst case is a redundant skip/retry.
        // The app being foregrounded is the common, deliberate case (the
        // foreground owns convergence) — `success`, not `retry`, so it doesn't
        // burn the retry budget.
        if (isAppForegrounded()) return Result.success()

        val configDir =
            inputData.getString(BackgroundSyncScheduler.KEY_CONFIG_DIR)
                ?: return Result.success() // stale work — nothing to do

        // AppLock-skip: the auth-free master key is absent when AppLock is on
        // (migrated to the biometric alias) or the store isn't set up. A
        // background worker can't show the biometric prompt, so skip cleanly.
        val keyB64 = MasterKeyAccess.loadAuthFree(applicationContext) ?: return Result.success()

        val json =
            try {
                nativeSync(configDir, keyB64)
            } catch (t: Throwable) {
                // Native panic / transient — retry with backoff, capped below.
                return if (runAttemptCount >= MAX_ATTEMPTS) Result.failure() else Result.retry()
            }

        return when (JSONObject(json).optString("status")) {
            "ok", "skipped" -> Result.success()
            "error" -> if (runAttemptCount >= MAX_ATTEMPTS) Result.failure() else Result.retry()
            else -> if (runAttemptCount >= MAX_ATTEMPTS) Result.failure() else Result.retry()
        }
    }

    /** Whether any process of this app is in the foreground. */
    private fun isAppForegrounded(): Boolean {
        val am = applicationContext.getSystemService(Context.ACTIVITY_SERVICE) as? ActivityManager
            ?: return false
        val processes = am.runningAppProcesses ?: return false
        return processes.any {
            it.importance <= ActivityManager.RunningAppProcessInfo.IMPORTANCE_FOREGROUND
        }
    }
}
