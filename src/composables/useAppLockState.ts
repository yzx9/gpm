// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { appLock, getAppLockState } from "../appLock";
import type { AppLockState } from "../types";

/**
 * Global app-launch biometric gate state — mirrors the backend `app-lock-state`
 * event and re-locks on app resume.
 *
 * The gate is independent of the identity cache lock (`useLockState`): it gates
 * the WHOLE store (the at-rest master key), not just the identity session. While
 * `appLocked` is true the app-lock overlay is shown and the identity
 * `UnlockModal` is suppressed, so the two never race to show competing prompts.
 *
 * Resume re-lock: the WebView fires `visibilitychange` when the Android activity
 * is backgrounded and resumed, so we re-challenge on every return to the
 * foreground (RFC 22's "every resume"). A loop guard (`unlockInFlight`) skips
 * the re-lock while a biometric prompt is already up, so the prompt's own
 * show/dismiss cannot re-trigger the gate.
 *
 * Module-scoped by design: one app-wide gate state, one event listener.
 */
const appLockEnabled = ref(false);
const appLocked = ref(false);
// False until `init()` has reconciled with the backend, so `App.vue` can avoid
// rendering the overlay during the brief boot window before the state is known.
const appReady = ref(false);

/// True while the overlay is driving an `app_unlock` biometric prompt. Suspends
/// the resume re-lock so the prompt can't re-lock itself.
let unlockInFlight = false;
/// Timestamp (ms) of the last locked→unlocked transition. The resume re-lock is
/// debounced for a short window after an unlock so the BiometricPrompt's own
/// show/dismiss — which on some OEM Android builds fires a `visibilitychange` —
/// can't immediately re-lock the app in a loop (RFC 22 loop guard). Standard
/// Android doesn't fire the event for the in-activity prompt, so this is
/// defense against the OEM edge case.
let lastUnlockAt = 0;
/// Resume-relock debounce window after an unlock, in milliseconds.
const APP_UNLOCK_DEBOUNCE_MS = 800;

let initialized = false;
let unlisten: UnlistenFn | null = null;

export function useAppLockState() {
  return {
    appLockEnabled,
    appLocked,
    appReady,
    init,
    setUnlockInFlight,
  };
}

/**
 * Reflect the backend's gate state, arm the single `app-lock-state` listener,
 * and start watching for app resume. Idempotent. Call once from `App.vue` on
 * mount. The backend is the single source of truth; this module never decides
 * state on its own.
 */
async function init() {
  if (initialized) return;
  initialized = true;

  unlisten ??= await listen<AppLockState>("app-lock-state", (e) =>
    onAppLockEvent(e.payload),
  );

  try {
    onAppLockEvent(await getAppLockState());
  } catch {
    // Couldn't read the gate state (pre-setup / desktop) — stay disabled.
    onAppLockEvent({ enabled: false, locked: false });
  }
  appReady.value = true;

  // Re-lock on resume. `visibilitychange→visible` fires when the Android
  // activity returns to the foreground (the WebView becomes visible again).
  document.addEventListener("visibilitychange", onVisibilityChange);
}

/** Backend gate-state event → the refs. */
function onAppLockEvent({ enabled, locked }: AppLockState) {
  const wasLocked = appLocked.value;
  appLockEnabled.value = enabled;
  appLocked.value = locked;
  // A locked→unlocked transition arms the post-unlock debounce (loop guard).
  if (wasLocked && !locked) {
    lastUnlockAt = Date.now();
  }
}

/**
 * Resume handler: if the gate is on and the app was unlocked, re-lock so the
 * user re-authenticates. Skipped when already locked (no-op), when the gate is
 * off, while a biometric prompt is in flight, or within the post-unlock debounce
 * window (so the prompt's own dismiss can't re-lock in a loop).
 */
function onVisibilityChange() {
  if (document.visibilityState !== "visible") return;
  if (!appLockEnabled.value || appLocked.value || unlockInFlight) return;
  if (Date.now() - lastUnlockAt < APP_UNLOCK_DEBOUNCE_MS) return;
  void appLock();
}

/** Mark a biometric app-unlock in flight (loop guard for the resume re-lock). */
function setUnlockInFlight(inFlight: boolean) {
  unlockInFlight = inFlight;
}

/** Test-only: reset the module singleton between cases. */
export function __resetAppLockStateForTests() {
  initialized = false;
  appLockEnabled.value = false;
  appLocked.value = false;
  appReady.value = false;
  unlockInFlight = false;
  lastUnlockAt = 0;
  unlisten?.();
  unlisten = null;
  document.removeEventListener("visibilitychange", onVisibilityChange);
}

/**
 * Test-only: put the app in the "gate enabled, locked" state page tests assume,
 * without mounting `App.vue` (which calls `init()` in production).
 */
export function __appLockEnabledLockedForTests() {
  initialized = true;
  appLockEnabled.value = true;
  appLocked.value = true;
  appReady.value = true;
}
