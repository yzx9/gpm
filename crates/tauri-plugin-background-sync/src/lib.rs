// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Tauri plugin that schedules the periodic Android background sync via
//! `WorkManager`, and cancels it when the cadence is turned `Off`.
//!
//! **Backend-only** from the capability standpoint: the frontend never calls
//! `plugin:background-sync|*` directly. App commands in `src-tauri/src/`
//! obtain the handle via [`BackgroundSyncExt`] and proxy. On non-Android
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

/// Android package hosting the `BackgroundSyncPlugin` Kotlin class.
#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "xyz.yzx9.gpm.backgroundsync";

/// Handle to the background-sync scheduler. On Android it wraps the mobile
/// plugin handle; on other targets it is an inert stub. `PhantomData<fn() -> R>`
/// keeps the stub `Send + Sync` unconditionally so it can live in app state.
#[cfg(target_os = "android")]
#[derive(Debug)]
pub struct BackgroundSync<R: Runtime>(PluginHandle<R>);

/// Handle to the background-sync scheduler — inert stub on non-Android targets.
///
/// `PhantomData<fn() -> R>` keeps the stub `Send + Sync` unconditionally so it
/// can live in app state on every target.
#[cfg(not(target_os = "android"))]
#[derive(Debug)]
pub struct BackgroundSync<R: Runtime>(PhantomData<fn() -> R>);

#[cfg(target_os = "android")]
impl<R: Runtime> BackgroundSync<R> {
    /// Enqueue/replace the periodic background-sync work at `interval_hours`,
    /// gated on `NetworkType.CONNECTED`. `config_dir` is forwarded as
    /// WorkManager `InputData` so the Worker reads it from there and never
    /// reconstructs the path (Rust is the single source of truth — D2).
    /// Idempotent (`ExistingPeriodicWorkPolicy::REPLACE` so a cadence change
    /// takes effect immediately). Best-effort: errors are swallowed (a missed
    /// re-schedule just keeps the previous cadence).
    pub async fn schedule(&self, interval_hours: u64, config_dir: String) {
        #[derive(Serialize)]
        struct Payload {
            #[serde(rename = "intervalHours")]
            interval_hours: u64,
            #[serde(rename = "configDir")]
            config_dir: String,
        }
        let _ = self
            .0
            .run_mobile_plugin_async::<()>(
                "schedule",
                Payload {
                    interval_hours,
                    config_dir,
                },
            )
            .await;
    }

    /// Cancel the periodic background-sync work (cadence turned `Off`).
    pub async fn cancel(&self) {
        let _ = self.0.run_mobile_plugin_async::<()>("cancel", ()).await;
    }
}

#[cfg(not(target_os = "android"))]
impl<R: Runtime> BackgroundSync<R> {
    /// Inert no-op (no `WorkManager` on desktop; the foreground sync covers it).
    #[expect(clippy::unused_async)]
    pub async fn schedule(&self, _interval_hours: u64, _config_dir: String) {}
    /// Inert no-op.
    #[expect(clippy::unused_async)]
    pub async fn cancel(&self) {}
}

/// Extension to access the background-sync handle from any [`Manager`] (e.g.
/// `AppHandle`).
pub trait BackgroundSyncExt<R: Runtime> {
    /// Obtain the background-sync scheduler handle. Always present; on
    /// non-Android targets it is an inert stub.
    fn background_sync_sched(&self) -> &BackgroundSync<R>;
}

impl<R: Runtime, T: Manager<R>> BackgroundSyncExt<R> for T {
    fn background_sync_sched(&self) -> &BackgroundSync<R> {
        self.state::<BackgroundSync<R>>().inner()
    }
}

/// Initializes the background-sync plugin.
///
/// On Android, registers the Kotlin `BackgroundSyncPlugin` and manages the
/// handle. On desktop, manages an inert stub so [`BackgroundSyncExt`] is always
/// callable.
#[must_use]
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("background-sync")
        .setup(|app, #[allow(unused_variables)] api| {
            #[cfg(target_os = "android")]
            {
                let handle =
                    api.register_android_plugin(PLUGIN_IDENTIFIER, "BackgroundSyncPlugin")?;
                app.manage(BackgroundSync(handle));
            }
            #[cfg(not(target_os = "android"))]
            {
                app.manage(BackgroundSync::<R>(PhantomData));
            }
            Ok(())
        })
        .build()
}
