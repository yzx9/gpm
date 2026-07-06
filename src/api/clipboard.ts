// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";

/**
 * Clipboard-clear notification permission IPC — mirrors
 * `src-tauri/src/clipboard.rs`. The notification is the sticky Android toast
 * shown while a password is on the clipboard so the user can clear it early;
 * these two commands drive the ask-once permission flow
 * ({@link ../composables/useClipboardNotify}). Desktop has no
 * notification-permission model — both report `true` there.
 */

/**
 * Whether the app may post notifications (Android 13+ runtime permission).
 * Cheap and non-prompting — the ask-once flow calls this before copying to
 * decide whether to prompt. Always `true` on desktop.
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
