// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Tauri plugin that posts a sticky Android notification while a secret is on
//! the clipboard, so the user can tap to clear it early without gpm
//! foregrounding.
//!
//! **Backend-only** from the capability standpoint: the frontend never calls
//! `plugin:clipboard-notify|*` directly. App commands in `src-tauri/src/`
//! obtain the handle via [`ClipboardNotifyExt`] and proxy. The notification's
//! tap is a broadcast that clears the clipboard natively and emits the
//! `clipboard-cleared` event so the Rust side can cancel the armed clear
//! timer (otherwise the timer would later clobber unrelated clipboard content
//! the user placed after the tap — see RFC 0037 + `.plans/0037`).
//!
//! On non-Android targets the plugin is registered but inert: every operation
//! is a no-op (`post`/`dismiss` return `Ok(())`, `are_enabled` reports `true`
//! so the frontend never prompts, `request_permission` reports `true`).

use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime};

// Serde + PluginHandle are only used inside the `#[cfg(target_os = "android")]`
// impl blocks; gate the imports so the desktop build doesn't see them as unused.
#[cfg(target_os = "android")]
use serde::{Deserialize, Serialize};
#[cfg(target_os = "android")]
use tauri::plugin::PluginHandle;

/// Android package hosting the `ClipboardNotifyPlugin` Kotlin class.
#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "xyz.yzx9.gpm.clipboardnotify";

// ---------------------------------------------------------------------------
// Handle (cfg-gated: real on Android, inert stub elsewhere)
// ---------------------------------------------------------------------------

/// Handle to the clipboard-notify plugin. On Android it wraps the mobile
/// plugin handle; on other targets it is an inert stub whose operations
/// succeed as no-ops. `PhantomData<fn() -> R>` keeps the stub `Send + Sync`
/// unconditionally so it can live in app state on every target.
#[cfg(target_os = "android")]
pub struct ClipboardNotify<R: Runtime>(PluginHandle<R>);

#[cfg(not(target_os = "android"))]
pub struct ClipboardNotify<R: Runtime>(std::marker::PhantomData<fn() -> R>);

#[cfg(target_os = "android")]
impl<R: Runtime> ClipboardNotify<R> {
    /// Whether the app may post notifications. Cheap, non-prompting.
    /// Reports `false` on plugin error so the frontend degrades to no
    /// notification rather than crashing the copy path.
    pub async fn are_enabled(&self) -> bool {
        #[derive(Deserialize)]
        struct Resp {
            enabled: bool,
        }
        self.0
            .run_mobile_plugin_async::<Resp>("areNotificationsEnabled", ())
            .await
            .map(|r| r.enabled)
            .unwrap_or(false)
    }

    /// Request `POST_NOTIFICATIONS` at runtime (Android 13+). Returns the
    /// grant state (always `true` on Android < 13). Holds the Kotlin `Invoke`
    /// across the system permission dialog.
    pub async fn request_permission(&self) -> bool {
        #[derive(Deserialize)]
        struct Resp {
            granted: bool,
        }
        self.0
            .run_mobile_plugin_async::<Resp>("requestNotificationsPermission", ())
            .await
            .map(|r| r.granted)
            .unwrap_or(false)
    }

    /// Post (or update, by fixed ID) the sticky clipboard-clear notification
    /// armed to fire `secs` as the displayed auto-clear window. Best-effort:
    /// errors are swallowed (a missing notification never fails a copy).
    pub async fn post_notification(&self, secs: u64) {
        #[derive(Serialize)]
        struct Payload {
            secs: u64,
        }
        let _ = self
            .0
            .run_mobile_plugin_async::<()>("postClipboardNotification", Payload { secs })
            .await;
    }

    /// Dismiss the sticky notification. Best-effort.
    pub async fn dismiss(&self) {
        let _ = self
            .0
            .run_mobile_plugin_async::<()>("dismissClipboardNotification", ())
            .await;
    }

    /// Atomically read + reset the manual-clear flag. The armed Rust clear
    /// timer calls this on wake: `true` means the user tapped the notification
    /// during the window (the receiver already cleared + dismissed), so the
    /// timer self-skips instead of clobbering unrelated clipboard content the
    /// user placed after the tap.
    pub async fn consume_manual_clear_flag(&self) -> bool {
        #[derive(Deserialize)]
        struct Resp {
            cleared: bool,
        }
        self.0
            .run_mobile_plugin_async::<Resp>("consumeManualClearFlag", ())
            .await
            .map(|r| r.cleared)
            .unwrap_or(false)
    }
}

#[cfg(not(target_os = "android"))]
impl<R: Runtime> ClipboardNotify<R> {
    /// Inert: always reports enabled so the frontend never prompts on desktop.
    pub async fn are_enabled(&self) -> bool {
        true
    }
    /// Inert: always reports granted on desktop.
    pub async fn request_permission(&self) -> bool {
        true
    }
    /// Inert no-op.
    pub async fn post_notification(&self, _secs: u64) {}
    /// Inert no-op.
    pub async fn dismiss(&self) {}
    /// Inert: reports no manual clear on desktop.
    pub async fn consume_manual_clear_flag(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// Extension trait
// ---------------------------------------------------------------------------

/// Extensions to access the clipboard-notify handle from any [`Manager`]
/// (e.g. `AppHandle`).
pub trait ClipboardNotifyExt<R: Runtime> {
    /// Obtain the clipboard-notify handle. Always present (the plugin is
    /// registered on every target); on non-Android targets the handle is an
    /// inert stub.
    fn clipboard_notify(&self) -> &ClipboardNotify<R>;
}

impl<R: Runtime, T: Manager<R>> ClipboardNotifyExt<R> for T {
    fn clipboard_notify(&self) -> &ClipboardNotify<R> {
        self.state::<ClipboardNotify<R>>().inner()
    }
}

// ---------------------------------------------------------------------------
// Plugin initialization
// ---------------------------------------------------------------------------

/// Initializes the clipboard-notify plugin.
///
/// On Android, registers the Kotlin `ClipboardNotifyPlugin` and manages the
/// handle. On desktop, manages an inert stub so `ClipboardNotifyExt` is always
/// callable.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("clipboard-notify")
        .setup(|app, #[allow(unused_variables)] api| {
            #[cfg(target_os = "android")]
            {
                let handle =
                    api.register_android_plugin(PLUGIN_IDENTIFIER, "ClipboardNotifyPlugin")?;
                app.manage(ClipboardNotify(handle));
            }
            #[cfg(not(target_os = "android"))]
            {
                app.manage(ClipboardNotify::<R>(std::marker::PhantomData));
            }
            Ok(())
        })
        .build()
}
