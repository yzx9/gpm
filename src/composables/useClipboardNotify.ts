// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

/**
 * Ask-once flow for the clipboard-clear notification permission (Android 13+).
 *
 * On the first copy after install, if notifications aren't enabled, prompt the
 * user once (gpm's confirm) to grant `POST_NOTIFICATIONS`; then proceed with
 * the copy regardless of the answer. The asked-flag lives in localStorage so
 * the prompt never repeats — if the user later grants via Android Settings,
 * the cheap `areNotificationsEnabled` check picks it up and the notification
 * resumes with no re-prompt. Desktop is a no-op (the check reports `true`).
 */

import {
  areClipboardNotificationsEnabled,
  requestClipboardNotificationsPermission,
} from "@/api";
import { i18n } from "@/i18n";

const ASKED_KEY = "gpm.clipboard.notify.asked";

/**
 * Ensure the clipboard-clear notification permission is asked-for at most once.
 * Call this before any copy path. After it resolves, proceed with the copy
 * whether or not the user granted — denial just means no notification (the
 * auto-clear timer still guards the clipboard).
 */
export async function ensureClipboardNotifyPermission(): Promise<void> {
  // The notification is a best-effort UX layer; a broken permission probe must
  // never brick the copy. Any error here degrades to "skip prompt, still copy".
  try {
    const enabled = await areClipboardNotificationsEnabled();
    if (enabled) return;
    if (localStorage.getItem(ASKED_KEY)) return;

    const wantGrant = window.confirm(
      i18n.global.t("common.clipboard.notifyPrompt"),
    );
    // Mark asked regardless of the answer so we never re-prompt. The user can
    // still grant later via Android Settings; the next copy's check sees it.
    localStorage.setItem(ASKED_KEY, "1");
    if (wantGrant) {
      // Fires the system permission dialog and awaits its result (the Kotlin
      // side holds the invoke across the dialog via requestPermissionForAlias).
      // Grant lands before this resolves; denial resolves with granted=false.
      await requestClipboardNotificationsPermission();
    }
  } catch {
    // Permission probe broken — degrade to "skip prompt, still copy".
  }
}
