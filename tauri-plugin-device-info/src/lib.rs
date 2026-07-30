// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Backend-only device-info plugin for gpm's diagnostics export: surfaces the
//! Android hardware/OS build fields (`Build.MANUFACTURER`/`MODEL`/`BRAND`,
//! `VERSION.SDK_INT`/`RELEASE`, `SUPPORTED_ABIS`), the WebView user-agent, and
//! the display metrics to Rust. Desktop gets a minimal OS/arch/version fallback
//! (it is a development surface, not the target).
//!
//! This is a **backend-only** plugin: the frontend never calls it directly. The
//! diagnostics-export command calls [`DeviceInfoExt::device_info`] to obtain the
//! handle, then [`DeviceInfoHandle::read`] to gather the snapshot.

#[cfg(target_os = "android")]
use tauri::plugin::mobile::PluginInvokeError;
use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime};

/// Android package hosting the `DeviceInfoPlugin` Kotlin class.
#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "xyz.yzx9.gpm.deviceinfo";

/// Display metrics snapshot (non-secret).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DisplayMetrics {
    /// Screen width in physical pixels.
    pub width_px: u32,
    /// Screen height in physical pixels.
    pub height_px: u32,
    /// Screen density in dots-per-inch.
    pub density_dpi: u32,
}

/// Device hardware/OS facts for the diagnostics bundle. All fields are
/// non-secret. The Android-specific fields are `None` on desktop.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DeviceInfo {
    /// `Build.MANUFACTURER` (Android only).
    pub manufacturer: Option<String>,
    /// `Build.MODEL` (Android only).
    pub model: Option<String>,
    /// `Build.BRAND` (Android only).
    pub brand: Option<String>,
    /// `Build.VERSION.SDK_INT` (Android only).
    pub sdk_int: Option<u32>,
    /// `Build.VERSION.RELEASE` (Android only).
    pub release: Option<String>,
    /// `Build.SUPPORTED_ABIS` (Android only).
    pub abis: Vec<String>,
    /// WebView user-agent (Android only).
    pub user_agent: Option<String>,
    /// Screen metrics (Android only).
    pub display: Option<DisplayMetrics>,
    /// `std::env::consts::OS` on every target.
    pub os: String,
    /// `std::env::consts::ARCH` on every target.
    pub arch: String,
    /// `env!("CARGO_PKG_VERSION")` (workspace version = the app version).
    pub app_version: String,
}

/// Error from device-info probing. Carries a machine-readable `code`
/// (`DEVICE_INFO_FAILED`, ...) and a safe (no-secret) message.
#[derive(Debug, Clone)]
pub struct DeviceInfoError {
    /// Machine-readable code, e.g. `DEVICE_INFO_FAILED`.
    pub code: String,
    /// Safe (no-secret) human-readable message.
    pub message: String,
}

/// Map a Tauri mobile-plugin invoke error into a [`DeviceInfoError`],
/// preserving the Kotlin-supplied code when present.
#[cfg(target_os = "android")]
fn map_invoke_err(err: PluginInvokeError) -> DeviceInfoError {
    match err {
        PluginInvokeError::InvokeRejected(resp) => DeviceInfoError {
            code: resp
                .code
                .unwrap_or_else(|| "DEVICE_INFO_FAILED".to_string()),
            message: resp
                .message
                .unwrap_or_else(|| "Device info probe failed".to_string()),
        },
        other => DeviceInfoError {
            code: "DEVICE_INFO_FAILED".to_string(),
            message: other.to_string(),
        },
    }
}

// ---------------------------------------------------------------------------
// DeviceInfoHandle (cfg-gated: mobile plugin handle on Android, inert stub
// elsewhere — desktop needs no native probe, it uses compile-time consts)
// ---------------------------------------------------------------------------

/// Handle to the device-info probe. On Android it wraps the mobile plugin
/// handle; on other targets it is an inert stub (desktop uses `std::env`).
#[cfg(target_os = "android")]
pub struct DeviceInfoHandle<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// Handle to the device-info probe — inert on non-Android targets.
#[cfg(not(target_os = "android"))]
pub struct DeviceInfoHandle<R: Runtime>(std::marker::PhantomData<fn() -> R>);

#[cfg(target_os = "android")]
impl<R: Runtime> DeviceInfoHandle<R> {
    /// Gather the device-info snapshot: Android build fields + WebView UA +
    /// display metrics from Kotlin, plus OS/arch/version from Rust.
    pub async fn read(&self) -> Result<DeviceInfo, DeviceInfoError> {
        #[derive(serde::Deserialize)]
        struct Resp {
            manufacturer: Option<String>,
            model: Option<String>,
            brand: Option<String>,
            sdk_int: Option<u32>,
            release: Option<String>,
            abis: Vec<String>,
            user_agent: Option<String>,
            display: Option<DisplayMetrics>,
        }

        let r: Resp = self
            .0
            .run_mobile_plugin_async("device_info", ())
            .await
            .map_err(map_invoke_err)?;

        Ok(DeviceInfo {
            manufacturer: r.manufacturer,
            model: r.model,
            brand: r.brand,
            sdk_int: r.sdk_int,
            release: r.release,
            abis: r.abis,
            user_agent: r.user_agent,
            display: r.display,
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
        })
    }
}

#[cfg(not(target_os = "android"))]
impl<R: Runtime> DeviceInfoHandle<R> {
    /// Gather the device-info snapshot: minimal OS/arch/version fallback
    /// (desktop is a development surface, not the target).
    pub async fn read(&self) -> Result<DeviceInfo, DeviceInfoError> {
        Ok(DeviceInfo {
            manufacturer: None,
            model: None,
            brand: None,
            sdk_int: None,
            release: None,
            abis: Vec::new(),
            user_agent: None,
            display: None,
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// Extension trait
// ---------------------------------------------------------------------------

/// Extensions to access the device-info handle from any [`Manager`]
/// (e.g. `AppHandle`).
pub trait DeviceInfoExt<R: Runtime> {
    /// Obtain the device-info handle. Always present (registered on every
    /// target).
    fn device_info(&self) -> &DeviceInfoHandle<R>;
}

impl<R: Runtime, T: Manager<R>> DeviceInfoExt<R> for T {
    fn device_info(&self) -> &DeviceInfoHandle<R> {
        self.state::<DeviceInfoHandle<R>>().inner()
    }
}

// ---------------------------------------------------------------------------
// Plugin initialization
// ---------------------------------------------------------------------------

/// Initializes the device-info plugin.
///
/// On Android, registers the Kotlin `DeviceInfoPlugin` and manages the handle.
/// On other targets, manages an inert handle that returns the OS/arch/version
/// fallback.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("device-info")
        .setup(|app, #[allow(unused_variables)] api| {
            #[cfg(target_os = "android")]
            {
                let handle = api.register_android_plugin(PLUGIN_IDENTIFIER, "DeviceInfoPlugin")?;
                app.manage(DeviceInfoHandle(handle));
            }
            #[cfg(not(target_os = "android"))]
            {
                app.manage(DeviceInfoHandle::<R>(std::marker::PhantomData));
            }
            Ok(())
        })
        .build()
}
