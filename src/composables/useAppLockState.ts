// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { ref, inject, type Ref, type InjectionKey } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { appLock, getAppLockState } from "@/appLock";
import type { AppLockState } from "@/types";

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
 * Provided app-wide via `APP_LOCK_KEY` (see `main.ts`): one instance, one event
 * listener. Tests construct their own via `createAppLockStore()`.
 */

/** The reactive app-launch gate state consumed by the UI. (Named `AppLockStore`
 *  to avoid clashing with the backend's `AppLockState` payload type.) */
export interface AppLockStore {
  readonly appLockEnabled: Readonly<Ref<boolean>>;
  readonly appLocked: Readonly<Ref<boolean>>;
  /** False until `init()` has reconciled with the backend. */
  readonly appReady: Readonly<Ref<boolean>>;
  /** Reflect backend gate state, arm the listener, watch for resume. Idempotent. */
  init: () => Promise<void>;
  /** Mark a biometric app-unlock in flight (loop guard for the resume re-lock). */
  setUnlockInFlight: (inFlight: boolean) => void;
  /** Tear down: drop the resume listener + Tauri subscription. A no-op for the
   *  production instance (one app lifetime); tests call it so the global
   *  `visibilitychange` listener doesn't leak across per-case instances. */
  dispose: () => void;
}

/** Seed options for `createAppLockStore` (test/seed only; production passes none). */
export interface CreateAppLockStateOptions {
  /**
   * Start in the "gate enabled, locked, ready" state (the precondition the old
   * `__appLockEnabledLockedForTests` fixture exposed). Default all-false.
   */
  enabledLocked?: boolean;
}

/** Resume-relock debounce window after an unlock, in milliseconds. */
const APP_UNLOCK_DEBOUNCE_MS = 800;

/** Injection key for the app-wide app-lock gate state. */
export const APP_LOCK_KEY: InjectionKey<AppLockStore> = Symbol("AppLockStore");

/**
 * Create a fresh app-lock gate instance. Production calls this once in `main.ts`
 * and provides it; tests call it per-case for isolation (no module singleton to
 * reset).
 */
export function createAppLockStore(
  opts: CreateAppLockStateOptions = {},
): AppLockStore {
  const el = opts.enabledLocked === true;
  const appLockEnabled = ref(el);
  const appLocked = ref(el);
  // False until `init()` has reconciled with the backend, so `App.vue` can avoid
  // rendering the overlay during the brief boot window before the state is known.
  const appReady = ref(el);

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

  let initialized = el;
  let unlisten: UnlistenFn | null = null;

  /**
   * Reflect the backend's gate state, arm the single `app-lock-state` listener,
   * and start watching for app resume. Idempotent. Call once from `App.vue` on
   * mount. The backend is the single source of truth; this instance never decides
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

  /** Remove the resume listener and the Tauri subscription (idempotent). */
  function dispose() {
    unlisten?.();
    unlisten = null;
    document.removeEventListener("visibilitychange", onVisibilityChange);
  }

  return {
    appLockEnabled,
    appLocked,
    appReady,
    init,
    setUnlockInFlight,
    dispose,
  };
}

/**
 * Inject the app-wide app-lock gate state. Must be called within a component
 * `setup()` under a tree that provided `APP_LOCK_KEY`. Throws if missing so a
 * forgotten `provide` fails loudly.
 */
export function useAppLockState(): AppLockStore {
  const s = inject(APP_LOCK_KEY);
  if (!s) {
    throw new Error("useAppLockState() requires APP_LOCK_KEY to be provided");
  }
  return s;
}
