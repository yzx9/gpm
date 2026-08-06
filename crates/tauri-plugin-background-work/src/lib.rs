// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Generic, app-agnostic periodic-WorkManager scheduler. The caller supplies
//! the worker class name, so this plugin carries **no** gpm-specific knowledge
//! — the headless worker + master-key retrieve live in the app, not here.
//!
//! **Backend-only** from the capability standpoint: the frontend never calls
//! `plugin:background-work|*` directly. App commands in `src-tauri/src/`
//! obtain the handle via [`BackgroundWorkExt`] and proxy. On non-Android
//! targets the plugin is registered but inert (schedule/cancel are no-ops;
//! the foreground sync covers desktop).

#[cfg(not(target_os = "android"))]
use std::marker::PhantomData;

use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime};

#[cfg(target_os = "android")]
use serde::Serialize;
#[cfg(target_os = "android")]
use tauri::plugin::PluginHandle;

/// Android package hosting the `BackgroundWorkPlugin` Kotlin class.
#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "xyz.yzx9.gpm.backgroundwork";

/// Handle to the periodic background-work scheduler. On Android it wraps the
/// mobile plugin handle; on other targets it is an inert stub.
/// `PhantomData<fn() -> R>` keeps the stub `Send + Sync` unconditionally so it
/// can live in app state.
#[cfg(target_os = "android")]
#[derive(Debug)]
pub struct BackgroundWork<R: Runtime>(PluginHandle<R>);

/// Handle to the periodic background-work scheduler — inert stub on
/// non-Android targets.
///
/// `PhantomData<fn() -> R>` keeps the stub `Send + Sync` unconditionally so it
/// can live in app state on every target.
#[cfg(not(target_os = "android"))]
#[derive(Debug)]
pub struct BackgroundWork<R: Runtime>(PhantomData<fn() -> R>);

#[cfg(target_os = "android")]
impl<R: Runtime> BackgroundWork<R> {
    /// Enqueue/replace the periodic background work at `interval_hours`, gated
    /// on `NetworkType.CONNECTED`, running the worker class named
    /// `worker_class_name` (FQN) and keyed by `work_name` (the unique
    /// WorkManager periodic-work name). `config_dir` is forwarded as WorkManager
    /// `InputData` so the Worker reads it from there and never reconstructs the
    /// path (Rust is the single source of truth). Idempotent
    /// (`ExistingPeriodicWorkPolicy::REPLACE` so a cadence change takes effect
    /// immediately). Best-effort: errors are swallowed (a missed re-schedule
    /// just keeps the previous cadence).
    pub async fn schedule(
        &self,
        interval_hours: u64,
        config_dir: String,
        worker_class_name: String,
        work_name: String,
    ) {
        #[derive(Serialize)]
        struct Payload {
            #[serde(rename = "intervalHours")]
            interval_hours: u64,
            #[serde(rename = "configDir")]
            config_dir: String,
            #[serde(rename = "workerClassName")]
            worker_class_name: String,
            #[serde(rename = "workName")]
            work_name: String,
        }
        let _ = self
            .0
            .run_mobile_plugin_async::<()>(
                "schedule",
                Payload {
                    interval_hours,
                    config_dir,
                    worker_class_name,
                    work_name,
                },
            )
            .await;
    }

    /// Cancel the periodic background work keyed by `work_name` (cadence `Off`).
    pub async fn cancel(&self, work_name: String) {
        #[derive(Serialize)]
        struct Payload {
            #[serde(rename = "workName")]
            work_name: String,
        }
        let _ = self
            .0
            .run_mobile_plugin_async::<()>("cancel", Payload { work_name })
            .await;
    }
}

#[cfg(not(target_os = "android"))]
impl<R: Runtime> BackgroundWork<R> {
    /// Inert no-op (no `WorkManager` on desktop; the foreground sync covers it).
    #[expect(clippy::unused_async)]
    pub async fn schedule(
        &self,
        _interval_hours: u64,
        _config_dir: String,
        _worker_class_name: String,
        _work_name: String,
    ) {
    }
    /// Inert no-op.
    #[expect(clippy::unused_async)]
    pub async fn cancel(&self, _work_name: String) {}
}

/// Extension to access the background-work handle from any [`Manager`] (e.g.
/// `AppHandle`).
pub trait BackgroundWorkExt<R: Runtime> {
    /// Obtain the background-work scheduler handle. Always present; on
    /// non-Android targets it is an inert stub.
    fn background_work_sched(&self) -> &BackgroundWork<R>;
}

impl<R: Runtime, T: Manager<R>> BackgroundWorkExt<R> for T {
    fn background_work_sched(&self) -> &BackgroundWork<R> {
        self.state::<BackgroundWork<R>>().inner()
    }
}

/// Initializes the background-work plugin.
///
/// On Android, registers the Kotlin `BackgroundWorkPlugin` and manages the
/// handle. On desktop, manages an inert stub so [`BackgroundWorkExt`] is always
/// callable.
#[must_use]
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("background-work")
        .setup(|app, #[allow(unused_variables)] api| {
            #[cfg(target_os = "android")]
            {
                let handle =
                    api.register_android_plugin(PLUGIN_IDENTIFIER, "BackgroundWorkPlugin")?;
                app.manage(BackgroundWork(handle));
            }
            #[cfg(not(target_os = "android"))]
            {
                app.manage(BackgroundWork::<R>(PhantomData));
            }
            Ok(())
        })
        .build()
}
