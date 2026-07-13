// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

/**
 * Idle-timer activity bumper — listens for user activity at the document level
 * and fires a throttled `bumpIdleTimer` so the backend's identity idle-lock
 * restarts on in-app use, not just on secret operations.
 *
 * Two filters short-circuit before any IPC (so Immediate/Never send zero bumps):
 * 1. `lockMode` must be `{ idle: n }`; 2. `identityCached` must be true.
 *
 * Leading-edge throttle (not a coalescing interval): the event itself fires the
 * bump, so a tap right before the timeout is caught immediately rather than
 * waiting for the next tick. Measured from the last *bump*, so a burst of
 * activity can lock up to `throttleMs` sooner than the configured idle window —
 * acceptable (the backend timer is authoritative; `throttleMs` is small vs. the
 * idle timeout). Via the reusable `throttle` util — `useAppLockState`'s inline
 * `lastUnlockAt` is a different shape (a one-shot post-unlock debounce) and
 * intentionally stays inline.
 *
 * Single-consumer: only `App.vue` constructs and inits this. Not provided; no
 * `useLockActivity()` accessor. Tests build their own via `createLockActivity()`.
 */
import type { Ref } from "vue";

import type { LockMode } from "@/api";
import { bumpIdleTimer } from "@/api";
import { throttle } from "@/utils/throttle";

export interface LockActivityOptions {
  /** Minimum spacing between bumps, ms. Default 5000. */
  throttleMs?: number;
}

export interface LockActivity {
  /** Attach the document listeners. Idempotent. */
  init: () => void;
  /** Detach the document listeners (test-only; production never unmounts). */
  dispose: () => void;
}

/** Minimum spacing between bumps while the user is continuously active. */
const DEFAULT_THROTTLE_MS = 5000;

/** Activity events. `input` covers soft-keyboard typing (incl. IME composition)
 *  where `keydown` is unreliable on Android. wheel/touchmove register
 *  `{ passive: true }` for scroll perf (matches usePullToRefresh); harmless on
 *  pointerdown/keydown/input. */
const EVENT_TYPES: (keyof DocumentEventMap)[] = [
  "pointerdown",
  "keydown",
  "wheel",
  "touchmove",
  "input",
];

/** Type guard for the `Idle` variant (the only object in the `LockMode` union).
 *  `!== null` because `typeof null === "object"` in JS. */
const isIdleMode = (m: LockMode): m is { idle: number } =>
  typeof m === "object" && m !== null;

/**
 * Create an idle-timer activity bumper. `lockMode` and `identityCached` are read
 * reactively at event time (not snapshotted), so a mid-session mode/cache change
 * takes effect without re-init.
 */
export function createLockActivity(
  lockMode: Readonly<Ref<LockMode>>,
  identityCached: Readonly<Ref<boolean>>,
  opts: LockActivityOptions = {},
): LockActivity {
  const throttleMs = opts.throttleMs ?? DEFAULT_THROTTLE_MS;
  let initialized = false;
  // Leading-edge throttle: first eligible activity bumps immediately, then ≤1
  // bump per throttleMs. The per-op resets on copy/show/write run server-side
  // and are independent; their overlap with a bump is a benign double-reset
  // (reset_lock_timer is idempotent).
  const bump = throttle(() => void bumpIdleTimer(), throttleMs);

  function onActivity() {
    // Filter 1: only Idle arms a backend timer worth bumping.
    if (!isIdleMode(lockMode.value)) return;
    // Filter 2: nothing to keep alive if the identity isn't cached (locked, or
    // mid-Immediate soft-wipe).
    if (!identityCached.value) return;
    bump();
  }

  function init() {
    if (initialized) return;
    initialized = true;
    for (const t of EVENT_TYPES) {
      document.addEventListener(t, onActivity, { passive: true });
    }
  }

  function dispose() {
    if (!initialized) return;
    initialized = false;
    // removeEventListener matches by reference + type + capture; the passive
    // flag is not part of the match key, so no options needed here.
    for (const t of EVENT_TYPES) {
      document.removeEventListener(t, onActivity);
    }
  }

  return { init, dispose };
}
