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
//! the user placed after the tap).
//!
//! On non-Android targets the plugin is registered but inert: every operation
//! is a no-op (`post`/`dismiss` return `Ok(())`, `are_enabled` reports `true`
//! so the frontend never prompts, `request_permission` reports `true`).

use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime};

// `Deserialize` is used unconditionally (the [`NotifyText`] IPC type deserializes
// on every target); `Serialize` + `PluginHandle` are Android-only (Payloads +
// the mobile handle).
use serde::Deserialize;
#[cfg(target_os = "android")]
use serde::Serialize;
#[cfg(target_os = "android")]
use tauri::plugin::PluginHandle;

/// Android package hosting the `ClipboardNotifyPlugin` Kotlin class.
#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "xyz.yzx9.gpm.clipboardnotify";

// ---------------------------------------------------------------------------
// Notification text
// ---------------------------------------------------------------------------

/// Localized clipboard-clear notification text supplied by the frontend, so the
/// native layer never localizes. `body_template` carries a `{secs}` hole
/// resolved against the auto-clear window at post time ([`Self::resolve_body`]).
/// Deserialized from the frontend's `{ title, bodyTemplate, channelName,
/// channelDescription }` shape (Tauri converts camelCase → snake_case at the
/// boundary, so the field names match).
#[derive(Debug, Clone, Deserialize)]
pub struct NotifyText {
    pub title: Option<String>,
    #[serde(rename = "bodyTemplate")]
    pub body_template: Option<String>,
    #[serde(rename = "channelName")]
    pub channel_name: Option<String>,
    #[serde(rename = "channelDescription")]
    pub channel_description: Option<String>,
}

impl NotifyText {
    /// Resolve the `{secs}` hole in `body_template` against the auto-clear
    /// window → the final notification body. Pure (no platform code), so it's
    /// unit-testable on desktop. `None` when no template was supplied (the
    /// native layer then falls back to a generic safety body).
    pub fn resolve_body(&self, secs: u64) -> Option<String> {
        self.body_template
            .as_ref()
            .map(|t| t.replace("{secs}", &secs.to_string()))
    }
}

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
    /// armed to fire `secs` as the displayed auto-clear window. `text` supplies
    /// the localized title/body/channel; the body template's `{secs}`
    /// hole is resolved here against `secs`. Best-effort: errors are swallowed
    /// (a missing notification never fails a copy).
    pub async fn post_notification(&self, secs: u64, text: Option<&NotifyText>) {
        #[derive(Serialize)]
        struct Payload {
            secs: u64,
            title: Option<String>,
            body: Option<String>,
            #[serde(rename = "channelName")]
            channel_name: Option<String>,
            #[serde(rename = "channelDescription")]
            channel_description: Option<String>,
        }
        let _ = self
            .0
            .run_mobile_plugin_async::<()>(
                "postClipboardNotification",
                Payload {
                    secs,
                    title: text.and_then(|x| x.title.clone()),
                    body: text.and_then(|x| x.resolve_body(secs)),
                    channel_name: text.and_then(|x| x.channel_name.clone()),
                    channel_description: text.and_then(|x| x.channel_description.clone()),
                },
            )
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
    pub async fn post_notification(&self, _secs: u64, _text: Option<&NotifyText>) {}
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

#[cfg(test)]
mod tests {
    use super::NotifyText;

    fn text(body_template: &str) -> NotifyText {
        NotifyText {
            title: None,
            body_template: Some(body_template.to_string()),
            channel_name: None,
            channel_description: None,
        }
    }

    #[test]
    fn resolve_body_substitutes_secs() {
        assert_eq!(
            text("Tap to clear · auto-clears in {secs}s")
                .resolve_body(45)
                .as_deref(),
            Some("Tap to clear · auto-clears in 45s"),
        );
    }

    #[test]
    fn resolve_body_none_when_no_template() {
        let n = NotifyText {
            title: None,
            body_template: None,
            channel_name: None,
            channel_description: None,
        };
        assert_eq!(n.resolve_body(45), None);
    }

    #[test]
    fn resolve_body_preserves_locale_word_order() {
        // zh-CN puts secs BEFORE the unit; the {secs} token is the only contract,
        // so word order is carried entirely by the template (Rust substitutes, it
        // doesn't reorder).
        assert_eq!(
            text("{secs} 秒后自动清除").resolve_body(60).as_deref(),
            Some("60 秒后自动清除"),
        );
    }
}
