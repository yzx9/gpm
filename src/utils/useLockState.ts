// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { ref, getCurrentScope, onScopeDispose } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AuthState } from "../types";

/**
 * Global lock state: the single source of truth for whether the identity is
 * locked, and the single consumer of the backend `identity-lock-state` event.
 *
 * `locked` drives the global `UnlockModal` overlay in `App.vue`. It defaults to
 * `true` (fail-closed) and is reconciled with the backend on `init()`.
 *
 * Any component that holds revealed/typed secret material in reactive state
 * registers a clearer via `onLock(cb)`; the callback fires the instant `locked`
 * flips to `true` — i.e. the same synchronous turn as the lock. This is the
 * explicit "clear secrets on lock" path the modal overlay needs (the old route
 * gave it for free by unmounting the page; the modal keeps pages mounted).
 *
 * Module-scoped by design: one app-wide lock state, one event listener.
 */
const locked = ref(true);
// False until `init()` has reconciled with the backend, so `App.vue` can avoid
// rendering the overlay during the brief boot window before we know the real
// state (default `locked` is fail-closed `true`).
const ready = ref(false);
let initialized = false;
const listeners = new Set<() => void>();
let unlisten: UnlistenFn | null = null;

export function useLockState() {
  return { locked, ready, init, setLocked };
}

/**
 * Register a callback to run whenever the identity becomes locked. Auto-removed
 * when the owning component/scope is disposed. Safe to call outside a scope
 * (e.g. in tests) — the callback is then only removable via the returned fn.
 *
 * @returns an unsubscribe function
 */
export function onLock(cb: () => void): () => void {
  listeners.add(cb);
  if (getCurrentScope()) {
    onScopeDispose(() => listeners.delete(cb));
  }
  return () => {
    listeners.delete(cb);
  };
}

/**
 * Reflect the backend's actual lock state and arm the single
 * `identity-lock-state` listener. Idempotent. Call once from `App.vue` on mount.
 *
 * The backend is the single source of truth: it emits `identity-lock-state`
 * `{ locked }` on every lock/unlock transition (timer, manual lock, unlock,
 * reset, setup). This ref only ever mirrors those events (+ the boot query) —
 * it must never be set by a component guessing the state.
 */
async function init() {
  if (initialized) return;
  initialized = true;

  // Own the one listener that mirrors the backend's lock-state snapshot.
  unlisten ??= await listen<{ locked: boolean }>("identity-lock-state", (e) =>
    setLocked(e.payload.locked),
  );

  try {
    const auth = await invoke<AuthState>("get_auth_state");
    // Unencrypted identities are never "locked" — there is nothing to unlock.
    setLocked(auth.encrypted && !auth.unlocked);
  } catch {
    // Couldn't read auth state — stay fail-closed.
    setLocked(true);
  }
  ready.value = true;
}

/**
 * Flip the lock state. When transitioning to locked, fire every registered
 * `onLock` clearer (each in its own try/catch so one failure can't mask the
 * others) *after* the ref has flipped, so the clear happens in the same
 * synchronous turn as the lock.
 */
function setLocked(v: boolean) {
  if (locked.value === v) return;
  locked.value = v;
  if (v) {
    for (const cb of [...listeners]) {
      try {
        cb();
      } catch {
        // A clearer must never block the others.
      }
    }
  }
}

/** Test-only: reset the module singleton between cases. */
export function __resetLockStateForTests() {
  initialized = false;
  locked.value = true;
  ready.value = false;
  listeners.clear();
  unlisten?.();
  unlisten = null;
}
