// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import {
  appLock,
  getAppLockState,
  subscribeAppLockState,
  subscribeAppResume,
  type AppLockReason,
  type AppLockState,
  type UnlistenFn,
} from "@/api";
import {
  computed,
  getCurrentScope,
  inject,
  onScopeDispose,
  ref,
  type ComputedRef,
  type InjectionKey,
  type Ref,
} from "vue";

/**
 * Global app-launch biometric gate state — mirrors the backend `app-lock-state`
 * event and re-locks on app resume.
 *
 * The gate is independent of the identity cache lock (`useLockState`): it gates
 * the WHOLE store (the seal master key), not just the identity session. While
 * `appLocked` is true the app-lock overlay is shown and the identity
 * `UnlockModal` is suppressed, so the two never race to show competing prompts.
 *
 * Resume re-lock: the backend emits the authoritative `app-resumed` signal from
 * `tauri::RunEvent::Resumed` (Android `Activity.onResume`, per tao) when the
 * activity resumes, so we re-challenge on every return to the foreground (RFC
 * 22's "every resume"). A loop guard (`unlockInFlight`) skips the re-lock while
 * a biometric prompt is already up, so the prompt's own show/dismiss cannot
 * re-trigger the gate.
 *
 * Provided app-wide via `APP_LOCK_KEY` (see `main.ts`): one instance, one event
 * listener. Tests construct their own via `createAppLockStore()`.
 */

/** The reactive app-launch gate state consumed by the UI. (Named `AppLockStore`
 *  to avoid clashing with the backend's `AppLockState` payload type.) */
export interface AppLockStore {
  readonly appLockEnabled: Readonly<Ref<boolean>>;
  readonly appLocked: Readonly<Ref<boolean>>;
  /** Whether the overlay should auto-fire the biometric prompt on mount. An idle
   *  re-lock suppresses it (the user is present but idle); cold start / resume
   *  keep it. Mirrors the identity `shouldAutoPromptBiometric`. */
  readonly shouldAutoPrompt: ComputedRef<boolean>;
  /** False until `init()` has reconciled with the backend. */
  readonly appReady: Readonly<Ref<boolean>>;
  /** Reflect backend gate state, arm the listener, watch for resume. Idempotent. */
  init: () => Promise<void>;
  /** Register a callback for the gate's unlock→locked edge — the gate mirror of
   *  identity's `onLock`, so eager-secret wipers can subscribe to both locks.
   *  Auto-removed on scope dispose. @returns an unsubscribe. */
  onAppLock: (cb: () => void) => () => void;
  /** Drive a gate state transition (public for tests; mirrors `setLocked`).
   *  Prefer this over firing the mocked `app-lock-state` listener — index-based
   *  handler capture drifts as a page's subscriptions grow. */
  setAppLocked: (locked: boolean, reason?: AppLockReason | null) => void;
  /** Mark a biometric app-unlock in flight (loop guard for the resume re-lock). */
  setUnlockInFlight: (inFlight: boolean) => void;
  /** Tear down: drop the resume listener + Tauri subscription. A no-op for the
   *  production instance (one app lifetime); tests call it so this instance's
   *  listeners don't leak across per-case instances. */
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
  /// Why the gate most recently locked (an AppLockReason) or null — recorded on
  /// a locked transition so the overlay can decide whether to auto-prompt.
  const gateLastReason = ref<AppLockReason | null>(null);
  /** Idle re-lock → suppress the auto-prompt; cold start (null) / return → keep. */
  const shouldAutoPrompt = computed(() => gateLastReason.value !== "idle");

  /// True while the overlay is driving an `app_unlock` biometric prompt. Suspends
  /// the resume re-lock so the prompt can't re-lock itself.
  let unlockInFlight = false;
  /// Timestamp (ms) of the last locked→unlocked transition. The resume re-lock is
  /// debounced for a short window after an unlock so the BiometricPrompt's own
  /// show/dismiss — which can drive an `Activity.onResume` (and thus an
  /// `app-resumed`) on some OEM builds — can't immediately re-lock the app in a
  /// loop (RFC 22 loop guard). Standard Android keeps the in-activity prompt off
  /// the host activity's lifecycle, so this is defense against the OEM edge case.
  let lastUnlockAt = 0;

  let initialized = el;
  let unlisten: UnlistenFn | null = null;
  /// Gate lock-edge callbacks (`onAppLock`) — fired on the unlock→locked
  /// transition only, mirroring `useLockState.onLock`'s listener set.
  const appLockListeners = new Set<() => void>();
  /// Unlisten handle for the authoritative resume signal (`subscribeAppResume`),
  /// torn down in `dispose()`. `disposed` closes the async-registration race: a
  /// late-resolving handle is released instead of leaking onto a disposed store.
  let resumeUnlisten: UnlistenFn | null = null;
  let disposed = false;

  /**
   * Reflect the backend's gate state, arm the single `app-lock-state` listener,
   * and start watching for app resume. Idempotent. Call once from `App.vue` on
   * mount. The backend is the single source of truth; this instance never decides
   * state on its own.
   */
  async function init() {
    if (initialized) return;
    initialized = true;

    unlisten ??= await subscribeAppLockState(onAppLockEvent);

    try {
      onAppLockEvent(await getAppLockState());
    } catch {
      // Couldn't read the gate state (pre-setup / desktop) — stay disabled.
      onAppLockEvent({ enabled: false, locked: false, reason: null });
    }
    appReady.value = true;

    // Re-lock on resume. The backend emits `app-resumed` from
    // `RunEvent::Resumed` when the Android activity returns to the foreground.
    const resumeHandle = await subscribeAppResume(onAppResume);
    if (disposed) {
      resumeHandle(); // disposed during the round-trip — release right away
    } else {
      resumeUnlisten = resumeHandle;
    }
  }

  /** Backend gate-state event → the refs. */
  function onAppLockEvent({ enabled, locked, reason }: AppLockState) {
    const wasLocked = appLocked.value;
    appLockEnabled.value = enabled;
    appLocked.value = locked;
    // Record why the gate locked (on a locked state) so the overlay can decide
    // whether to auto-fire the biometric prompt. `idle` suppresses it.
    if (locked) {
      gateLastReason.value = reason ?? null;
    }
    // A locked→unlocked transition arms the post-unlock debounce (loop guard).
    if (wasLocked && !locked) {
      lastUnlockAt = Date.now();
    }
    // Fire the gate-lock clearers on the unlock→locked edge, after the ref flip
    // (same synchronous turn — mirrors `useLockState.setLocked`). The cold-start
    // reconcile also lands here (false→locked); harmless: wipers are idempotent
    // and a freshly mounted page holds nothing. Locked→locked emits never fire
    // (the backend doesn't emit them, and the edge check drops them anyway).
    if (locked && !wasLocked) {
      for (const cb of [...appLockListeners]) {
        try {
          cb();
        } catch {
          // A clearer must never block the others.
        }
      }
    }
  }

  /**
   * Register a callback for the gate's unlock→locked edge (idle re-lock, resume
   * re-lock, cold-start reconcile). The gate mirror of identity's `onLock` —
   * eager-secret wipers subscribe to both so a gate re-lock also clears in-DOM
   * secrets (issue #20). Safe to call outside a scope (tests); then only the
   * returned fn removes it.
   *
   * @returns an unsubscribe function
   */
  function onAppLock(cb: () => void): () => void {
    appLockListeners.add(cb);
    if (getCurrentScope()) {
      onScopeDispose(() => appLockListeners.delete(cb));
    }
    return () => {
      appLockListeners.delete(cb);
    };
  }

  /** Test driver: same path as the backend event (see the interface doc). */
  function setAppLocked(locked: boolean, reason: AppLockReason | null = null) {
    onAppLockEvent({ enabled: appLockEnabled.value, locked, reason });
  }

  /**
   * Resume handler: if the gate is on and the app was unlocked, ping the backend
   * `app_lock` (R058 grace-aware) so it can re-lock past the grace window. Skipped
   * when the gate is off, when already locked (the backend's `apply_resume_relock`
   * is a no-op then — and skipping avoids a spurious cold-start ping that could
   * race a just-finished unlock), while a biometric prompt is in flight, or within
   * the post-unlock debounce window (loop guard).
   */
  function onAppResume() {
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
    disposed = true;
    unlisten?.();
    unlisten = null;
    resumeUnlisten?.();
    resumeUnlisten = null;
    appLockListeners.clear();
  }

  return {
    appLockEnabled,
    appLocked,
    shouldAutoPrompt,
    appReady,
    init,
    onAppLock,
    setAppLocked,
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
