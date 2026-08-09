// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/**
 * App-lifecycle signals bridged from the native event loop to the WebView.
 *
 * Unlike the DOM `visibilitychange`/`focus` events (WebView-layer, which some
 * OEM Android builds fail to fire, so a resume can silently fail to re-engage
 * the app lock), these come from below the WebView: the backend emits
 * {@link appResumeEventName} from `tauri::RunEvent::Resumed`, which tao
 * documents as "Android: triggered by `onResume` of the Activity" — the
 * platform-guaranteed foreground transition (R029). The resume consumers
 * (`useAppLockState`, `useForegroundSync`, `SettingsPermissionsPage`) subscribe
 * here instead of the DOM event.
 *
 * This is pattern A (a Rust-`emit`-ed global event → `listen`/`UnlistenFn`),
 * like `subscribeAppLockState`, NOT a plugin-IPC event, so it lives in its own
 * module (not `system.ts`, which is the plugin-IPC home).
 */

/** The global event the backend emits on app foreground-return. MUST match the
 *  `APP_RESUME_EVENT` literal in `src-tauri/src/lib.rs` (`on_run_event`). */
export const appResumeEventName = "app-resumed";

/**
 * Subscribe to the authoritative app-resume signal. Fires once per
 * `Activity.onResume` on Android (cold start included — the consumers' guards
 * no-op then). Desktop: `RunEvent::Resumed` fires at event-loop start, not on
 * tab/window focus, so this rarely fires there — by design (no app lock on
 * desktop; gpm is Android-first). Returns an unlisten handle.
 */
export async function subscribeAppResume(cb: () => void): Promise<UnlistenFn> {
  return listen(appResumeEventName, () => cb());
}
