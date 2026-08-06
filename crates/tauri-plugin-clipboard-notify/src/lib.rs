// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Tauri plugin that posts a sticky Android notification while a secret is on
//! the clipboard, so the user can tap to clear it early without bringing the
//! host app to the foreground.
//!
//! **Backend-only** from the capability standpoint: the frontend never calls
//! `plugin:clipboard-notify|*` directly. App commands in `src-tauri/src/`
//! obtain the handle via [`ClipboardNotifyExt`] and proxy. The notification's
//! tap is a manifest-declared broadcast that clears the clipboard natively and
//! sets a manual-clear flag; the Rust armed clear timer **polls** that flag on
//! wake (via [`ClipboardNotify::consume_manual_clear_flag`]) and self-skips if
//! the user already cleared — so it cannot later clobber unrelated clipboard
//! content the user placed after the tap. There is no Kotlin→Rust event; the
//! flag is polled over the proven `run_mobile_plugin_async` direction.
//!
//! On non-Android targets the plugin is registered but inert: every operation
//! is a no-op (`post`/`dismiss` return `Ok(())`, `are_enabled` reports `true`
//! so the frontend never prompts, `request_permission` reports `true`).

#[cfg(not(target_os = "android"))]
use std::marker::PhantomData;

use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime};

// `Deserialize`/`Serialize` are used unconditionally (the [`NotifyText`] IPC
// type deserializes on every target; `ResolvedNotificationText` serializes on
// every target so its construction is unit-testable); `PluginHandle` is
// Android-only (the mobile handle).
use serde::{Deserialize, Serialize};
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
/// channelDescription }` shape (Tauri converts camelCase → `snake_case` at the
/// boundary, so the field names match).
#[derive(Debug, Clone, Deserialize)]
pub struct NotifyText {
    /// Notification title.
    pub title: Option<String>,
    /// Notification body template carrying a `{secs}` hole resolved against the
    /// auto-clear window at post time.
    #[serde(rename = "bodyTemplate")]
    pub body_template: Option<String>,
    /// Android notification channel display name.
    #[serde(rename = "channelName")]
    pub channel_name: Option<String>,
    /// Android notification channel description (shown in system settings).
    #[serde(rename = "channelDescription")]
    pub channel_description: Option<String>,
}

impl NotifyText {
    /// Resolve the `{secs}` hole in `body_template` against the auto-clear
    /// window → the final notification body. Pure (no platform code), so it's
    /// unit-testable on desktop. `None` when no template was supplied (the
    /// native layer then falls back to a generic safety body).
    #[must_use]
    pub fn resolve_body(&self, secs: u64) -> Option<String> {
        self.body_template
            .as_ref()
            .map(|t| t.replace("{secs}", &secs.to_string()))
    }
}

/// [`NotifyText`] with caller-supplied fallbacks applied — every field is
/// non-empty, and the body's `{secs}` hole is substituted. Passed to
/// [`ClipboardNotify::post_notification`]. The plugin never bakes a brand string;
/// the app supplies the fallback values (e.g. its own app name).
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedNotificationText {
    /// Notification title (non-empty).
    pub title: String,
    /// Notification body with `{secs}` substituted (non-empty).
    pub body: String,
    /// Notification channel display name (non-empty).
    #[serde(rename = "channelName")]
    pub channel_name: String,
    /// Notification channel description (non-empty).
    #[serde(rename = "channelDescription")]
    pub channel_description: String,
}

/// Resolve [`NotifyText`] against caller-supplied fallbacks. Pure (no platform
/// code). A blank provided field falls back; the body template's `{secs}` hole is
/// substituted against `secs` in BOTH the provided template and the fallback.
/// `text = None` resolves every field to its fallback (the frontend omitted the
/// localized text). The plugin carries no brand string — the app supplies the
/// fallbacks.
#[must_use]
pub fn resolve_notification_text(
    text: Option<&NotifyText>,
    secs: u64,
    fallback_title: &str,
    fallback_body: &str,
    fallback_channel_name: &str,
    fallback_channel_description: &str,
) -> ResolvedNotificationText {
    /// Pick a non-blank provided field, else the fallback.
    fn pick(provided: Option<&str>, fallback: &str) -> String {
        provided
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(fallback)
            .to_owned()
    }
    /// Like [`pick`], but also substitute `{secs}` (the body template).
    fn pick_secs(provided: Option<&str>, fallback: &str, secs: u64) -> String {
        let chosen = provided
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(fallback);
        chosen.replace("{secs}", &secs.to_string())
    }
    ResolvedNotificationText {
        title: pick(text.and_then(|t| t.title.as_deref()), fallback_title),
        body: pick_secs(
            text.and_then(|t| t.body_template.as_deref()),
            fallback_body,
            secs,
        ),
        channel_name: pick(
            text.and_then(|t| t.channel_name.as_deref()),
            fallback_channel_name,
        ),
        channel_description: pick(
            text.and_then(|t| t.channel_description.as_deref()),
            fallback_channel_description,
        ),
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
#[derive(Debug)]
pub struct ClipboardNotify<R: Runtime>(PluginHandle<R>);

/// Handle to the clipboard-notify plugin — inert stub on non-Android targets
/// whose operations succeed as no-ops. `PhantomData<fn() -> R>` keeps the stub
/// `Send + Sync` unconditionally so it can live in app state on every target.
#[cfg(not(target_os = "android"))]
#[derive(Debug)]
pub struct ClipboardNotify<R: Runtime>(PhantomData<fn() -> R>);

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

    /// Open the system's per-app notification-settings screen — the recovery
    /// surface when the runtime `POST_NOTIFICATIONS` dialog is suppressed after
    /// two denials. Returns whether a handler activity was found and started
    /// (`false` on the rare OEM ROM lacking the target), so the caller can toast
    /// instead of failing silently.
    pub async fn open_notification_settings(&self) -> bool {
        #[derive(Deserialize)]
        struct Resp {
            opened: bool,
        }
        self.0
            .run_mobile_plugin_async::<Resp>("openAppNotificationSettings", ())
            .await
            .map(|r| r.opened)
            .unwrap_or_else(|e| {
                // `opened: false` from the Kotlin catch (no handler activity) is
                // expected; a plugin-invoke failure here is not, so log it before
                // collapsing to false — otherwise the recovery tap fails silently.
                log::warn!("open_notification_settings: plugin invoke failed: {e:?}");
                false
            })
    }

    /// Post (or update, by fixed ID) the sticky clipboard-clear notification
    /// armed to fire `secs` as the displayed auto-clear window. `text` is the
    /// already-resolved notification text (see [`resolve_notification_text`]) —
    /// the plugin applies no fallback of its own. Best-effort: errors are
    /// swallowed (a missing notification never fails a copy).
    pub async fn post_notification(&self, secs: u64, text: &ResolvedNotificationText) {
        #[derive(Serialize)]
        struct Payload {
            secs: u64,
            title: String,
            body: String,
            #[serde(rename = "channelName")]
            channel_name: String,
            #[serde(rename = "channelDescription")]
            channel_description: String,
        }
        let _ = self
            .0
            .run_mobile_plugin_async::<()>(
                "postClipboardNotification",
                Payload {
                    secs,
                    title: text.title.clone(),
                    body: text.body.clone(),
                    channel_name: text.channel_name.clone(),
                    channel_description: text.channel_description.clone(),
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
    #[expect(clippy::unused_async)]
    pub async fn are_enabled(&self) -> bool {
        true
    }
    /// Inert: always reports granted on desktop.
    #[expect(clippy::unused_async)]
    pub async fn request_permission(&self) -> bool {
        true
    }
    /// Inert: nothing to open on desktop; reports `true` so a (never-shown on
    /// desktop) row never toasts a spurious failure.
    #[expect(clippy::unused_async)]
    pub async fn open_notification_settings(&self) -> bool {
        true
    }
    /// Inert no-op.
    #[expect(clippy::unused_async)]
    pub async fn post_notification(&self, _secs: u64, _text: &ResolvedNotificationText) {}
    /// Inert no-op.
    #[expect(clippy::unused_async)]
    pub async fn dismiss(&self) {}
    /// Inert: reports no manual clear on desktop.
    #[expect(clippy::unused_async)]
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
#[must_use]
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
                app.manage(ClipboardNotify::<R>(PhantomData));
            }
            Ok(())
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::{NotifyText, resolve_notification_text};

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

    #[test]
    fn resolve_notification_text_none_uses_all_fallbacks() {
        // No NotifyText at all → every field is its caller fallback. The body
        // fallback's {secs} (if any) is substituted; a plain fallback passes thru.
        let r = resolve_notification_text(None, 45, "MyApp", "Tap to clear", "MyApp", "MyApp desc");
        assert_eq!(r.title, "MyApp");
        assert_eq!(r.body, "Tap to clear");
        assert_eq!(r.channel_name, "MyApp");
        assert_eq!(r.channel_description, "MyApp desc");
    }

    #[test]
    fn resolve_notification_text_keeps_provided_and_substitutes_secs() {
        let t = NotifyText {
            title: Some("Custom".to_owned()),
            body_template: Some("Clears in {secs}s".to_owned()),
            channel_name: Some("  ".to_owned()), // blank → fallback
            channel_description: None,
        };
        let r =
            resolve_notification_text(Some(&t), 30, "MyApp", "Tap to clear", "MyApp", "MyApp desc");
        assert_eq!(r.title, "Custom");
        assert_eq!(r.body, "Clears in 30s");
        assert_eq!(r.channel_name, "MyApp"); // blank fell back
        assert_eq!(r.channel_description, "MyApp desc");
    }

    #[test]
    fn resolve_notification_text_substitutes_secs_in_fallback_body() {
        // A fallback body template carries {secs} too.
        let r = resolve_notification_text(None, 12, "App", "Auto-clears in {secs}s", "App", "App");
        assert_eq!(r.body, "Auto-clears in 12s");
    }

    /// The `notify_text: Option<NotifyText>` command arg is deserialized from the
    /// frontend's camelCase object at the IPC boundary — pin that contract so a
    /// future field-rename or a string-vs-struct type drift on either side fails
    /// here instead of surfacing at runtime as `invalid type: map, expected a
    /// string` (or vice-versa).
    #[test]
    fn notify_text_deserializes_from_frontend_ipc_payload() {
        // Exact shape `clipboardNotifyText()` sends (see app/src/i18n/native.ts).
        let payload = serde_json::json!({
            "title": "gpm",
            "bodyTemplate": "Tap to clear · auto-clears in {secs}s",
            "channelName": "Clipboard",
            "channelDescription": "Notifies when a secret is on the clipboard so you can clear it"
        });
        let n: NotifyText =
            serde_json::from_value(payload).expect("frontend payload must map to NotifyText");
        assert_eq!(n.title.as_deref(), Some("gpm"));
        assert_eq!(
            n.body_template.as_deref(),
            Some("Tap to clear · auto-clears in {secs}s")
        );
        assert_eq!(n.channel_name.as_deref(), Some("Clipboard"));
        assert_eq!(
            n.channel_description.as_deref(),
            Some("Notifies when a secret is on the clipboard so you can clear it")
        );

        // The `notify?` (optional) case: an absent/null arg resolves to `None`.
        assert!(
            serde_json::from_value::<Option<NotifyText>>(serde_json::Value::Null)
                .unwrap()
                .is_none()
        );
    }
}
