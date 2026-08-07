// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { invoke } from "@tauri-apps/api/core";

/**
 * Clipboard-clear notification permission IPC — mirrors
 * `src-tauri/src/clipboard.rs`. The notification is the sticky Android toast
 * shown while a password is on the clipboard so the user can clear it early.
 * Desktop has no notification-permission model — both commands report `true`
 * there, so {@link ensureClipboardNotifyPermission} is a no-op on desktop.
 */

/**
 * Whether the app may post notifications (Android 13+ runtime permission).
 * Cheap and non-prompting — callers check this before copying to skip the
 * system dialog when already granted. Always `true` on desktop.
 */
export async function areClipboardNotificationsEnabled(): Promise<boolean> {
  return invoke<boolean>("are_clipboard_notifications_enabled");
}

/**
 * Request `POST_NOTIFICATIONS` at runtime (Android 13+). Shows the system
 * dialog and returns the grant state. Always `true` on desktop.
 */
export async function requestClipboardNotificationsPermission(): Promise<boolean> {
  return invoke<boolean>("request_clipboard_notifications_permission");
}

/**
 * Open the system's per-app notification-settings screen — the recovery surface
 * when Android has suppressed the runtime `POST_NOTIFICATIONS` dialog after two
 * denials (the only way back to re-enabling the clipboard-clear notification).
 * Returns whether a handler activity was found and started; `false` (or a throw)
 * means the page should toast, not fail silently. Always `true` on desktop.
 */
export async function openClipboardNotificationSettings(): Promise<boolean> {
  return invoke<boolean>("open_clipboard_notification_settings");
}

/**
 * Before a copy, request the clipboard-clear notification permission via the
 * system `POST_NOTIFICATIONS` dialog whenever the enabled-probe reports false
 * (so every copy that finds notifications disabled fires it, until Android's
 * own two-denial suppression takes over). Best-effort: any failure degrades to
 * "skip, still copy" — the notification is a UX affordance and never gates the
 * copy (the auto-clear timer is the independent security control). No-op on
 * desktop.
 */
export async function ensureClipboardNotifyPermission(): Promise<void> {
  try {
    if (await areClipboardNotificationsEnabled()) return;
    await requestClipboardNotificationsPermission();
  } catch (e) {
    // A broken probe must never block the copy — degrade, but log the
    // unexpected failure so a missing plugin or Kotlin crash stays diagnosable.
    console.warn("clipboard notify permission probe failed", e);
  }
}
