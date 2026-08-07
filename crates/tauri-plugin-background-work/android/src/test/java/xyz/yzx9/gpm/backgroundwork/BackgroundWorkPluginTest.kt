// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

package xyz.yzx9.gpm.backgroundwork

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import com.fasterxml.jackson.databind.DeserializationFeature
import com.fasterxml.jackson.databind.ObjectMapper
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Pins the worker-agnostic scheduler's two contracts:
 *
 * 1. **IPC shape** — the Rust `Payload` emits a flat `workerClassName` field;
 *    Tauri parses `@InvokeArg` via Jackson with `FAIL_ON_UNKNOWN_PROPERTIES`
 *    disabled, so a field-name mismatch would silently null
 *    [ScheduleArgs.workerClassName] and the schedule would no-op. The fast host
 *    gates can't see this Kotlin↔Rust shape, so it is pinned here (mirrors the
 *    keystore plugin's contract test).
 * 2. **FQN resolution** — `schedule` resolves the caller-supplied worker FQN to
 *    a [ListenableWorker] subclass via `Class.forName` + `asSubclass`; a wrong
 *    FQN throws (surfaced by the plugin as `BG_WORKER_CLASS`).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class BackgroundWorkPluginTest {

    private val tauriLikeMapper = ObjectMapper()
        .disable(DeserializationFeature.FAIL_ON_UNKNOWN_PROPERTIES)

    @Test
    fun scheduleArgs_bindsFlatFieldsFromRustPayload() {
        // The exact JSON the Rust `Payload` serializes (intervalHours / configDir
        // / workerClassName / workName, all flat camelCase — a nested object or a
        // renamed key would silently null the field and the schedule would no-op).
        val json =
            """{"intervalHours":6,"configDir":"/data/user/0/xyz.yzx9.gpm/files","workerClassName":"xyz.yzx9.gpm.SyncWorker","workName":"gpm_background_sync"}"""
        val args = tauriLikeMapper.readValue(json, ScheduleArgs::class.java)
        assertEquals(6L, args.intervalHours)
        assertEquals("/data/user/0/xyz.yzx9.gpm/files", args.configDir)
        assertEquals("xyz.yzx9.gpm.SyncWorker", args.workerClassName)
        assertEquals("gpm_background_sync", args.workName)
    }

    @Test
    fun cancelArgs_bindsWorkNameFromRustPayload() {
        // The Rust cancel `Payload` emits a flat workName (the unique-work name
        // to cancel). A field-name mismatch would silently null it → the plugin
        // rejects as BG_WORK_ARGS and cancel no-ops.
        val json = """{"workName":"gpm_background_sync"}"""
        val args = tauriLikeMapper.readValue(json, CancelArgs::class.java)
        assertEquals("gpm_background_sync", args.workName)
    }

    @Test
    fun scheduleArgs_optionalFieldsDefaultNullWhenAbsent() {
        // Characterization: a Rust payload without workerClassName/workName
        // leaves them null — the plugin's empty-check then rejects. Pins that
        // nothing else implicitly populates them.
        val json = """{"intervalHours":6,"configDir":"/d"}"""
        val args = tauriLikeMapper.readValue(json, ScheduleArgs::class.java)
        assertNull(args.workerClassName)
        assertNull(args.workName)
    }

    @Test
    fun schedule_rejectsUnknownWorkerFqn() {
        // Drives the PRODUCTION path (not the Class.forName primitive): the
        // scheduler resolves the FQN as its first statement, so a wrong/stale
        // name throws ClassNotFoundException before reaching WorkManager — no
        // WM/Robolectric init needed. The plugin wraps this into BG_WORKER_CLASS.
        try {
            BackgroundWorkScheduler.schedule(
                ApplicationProvider.getApplicationContext<Context>(),
                6L,
                "/d",
                "xyz.yzx9.gpm.DoesNotExist",
                "test_work",
            )
            org.junit.Assert.fail("expected ClassNotFoundException for an unknown worker FQN")
        } catch (e: ClassNotFoundException) {
            // expected — scheduler's Class.forName threw before enqueue
        }
    }

    @Test
    fun schedule_acceptsValidWorkerFqn() {
        // A real ListenableWorker FQN resolves past Class.forName + asSubclass +
        // the PeriodicWorkRequest builder; WorkManager.getInstance then throws
        // (not initialized under Robolectric) — proof the FQN resolved. The
        // app's own SyncWorker is verified end-to-end by the device build.
        try {
            BackgroundWorkScheduler.schedule(
                ApplicationProvider.getApplicationContext<Context>(),
                6L,
                "/d",
                "androidx.work.Worker",
                "test_work",
            )
            org.junit.Assert.fail("expected IllegalStateException (WorkManager not initialized under Robolectric)")
        } catch (e: IllegalStateException) {
            // expected — FQN resolved; WM init threw under Robolectric
        } catch (e: ClassNotFoundException) {
            org.junit.Assert.fail("androidx.work.Worker must resolve on the classpath")
        }
    }
}
