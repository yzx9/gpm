// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { ref, computed, getCurrentScope, onScopeDispose } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AuthState } from "../types";

/**
 * Global lock state — the single source of truth for the identity's runtime
 * state, and the single consumer of the backend `identity-lock-state` event.
 *
 * The model splits two concerns that used to be one:
 * - `locked` — is the _hard-lock_ overlay up? (manual lock or idle-timeout). It
 *   drives the global `UnlockModal` and fires the `onLock` clearers. Mirrors the
 *   backend's hard transitions only.
 * - `identityCached` — is the decrypted identity in the backend cache, i.e. will
 *   the next identity-needing operation succeed without re-authenticating? In a
 *   session (Idle/Never) mode this tracks `locked`; in Immediate (no-cache) mode
 *   the two diverge — a soft wipe empties the cache (→ `identityCached = false`)
 *   without raising the overlay (→ `locked` stays false), so a just-revealed
 *   password stays on screen.
 *
 * `overlayUp` is what the UI gates the modal on: `locked || authPrompted`
 * (the latter is a per-operation auth prompt, which also shows the modal but
 * must NOT fire `onLock` — clearing the page mid-op would lose a draft or the
 * reveal being authenticated for).
 *
 * Module-scoped by design: one app-wide lock state, one event listener.
 */
const locked = ref(true);
// False until `init()` has reconciled with the backend, so `App.vue` can avoid
// rendering the overlay during the brief boot window before we know the real
// state (default `locked` is fail-closed `true`).
const ready = ref(false);
// Whether the decrypted identity is in the backend cache (next op needs no
// auth). Plaintext identities report `true` (they decrypt straight from disk).
const identityCached = ref(false);
// Overlay is up specifically for a per-operation auth (Immediate mode), not a
// hard lock. Drives `overlayUp` without firing `onLock`.
const authPrompted = ref(false);

/// The shared promise awaiting op callers park on while the per-op auth overlay
/// is up. All concurrent callers await the same one (single-flight: one prompt,
/// not N). Resolved when the backend reports an unlock.
let authPromise: Promise<void> | null = null;
let resolveAuth: (() => void) | null = null;
// Reject fn paired with `authPromise` — invoked by `cancelAuth()` to abort every
// parked caller when the user dismisses the per-op auth overlay (e.g. Android back).
let rejectAuth: ((e: unknown) => void) | null = null;

let initialized = false;
const listeners = new Set<() => void>();
let unlisten: UnlistenFn | null = null;

/** Whether the unlock overlay should be shown (hard lock OR per-op auth). */
const overlayUp = computed(() => locked.value || authPrompted.value);

export function useLockState() {
  return {
    locked,
    overlayUp,
    identityCached,
    ready,
    init,
    setLocked,
    cancelAuth,
  };
}

/**
 * Register a callback to run whenever the identity becomes _hard_-locked (manual
 * / idle). Auto-removed when the owning component/scope is disposed. Safe to
 * call outside a scope (e.g. in tests) — the callback is then only removable via
 * the returned fn. NOTE: a soft wipe (Immediate post-op) does NOT fire this — it
 * must not clear a revealed secret or a create-form draft.
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
 * `{ locked, soft }` on every transition. `locked` reports whether the identity
 * is NOT cached; `soft` marks a soft wipe (Immediate post-op) that must leave the
 * overlay down. This module never decides state on its own.
 */
async function init() {
  if (initialized) return;
  initialized = true;

  // Own the one listener that mirrors the backend's lock-state snapshot.
  unlisten ??= await listen<{ locked: boolean; soft: boolean }>(
    "identity-lock-state",
    (e) => onLockEvent(e.payload),
  );

  try {
    const auth = await invoke<AuthState>("get_auth_state");
    // Unencrypted identities are never "locked" — there is nothing to unlock.
    // setLocked mirrors identityCached: encrypted+locked ⇒ not cached,
    // otherwise cached (plaintext always reads straight from disk).
    setLocked(auth.encrypted && !auth.unlocked);
  } catch {
    // Couldn't read auth state — stay fail-closed.
    setLocked(true);
  }
  ready.value = true;
}

/** Backend lock-state event → the two refs. Soft wipes touch only the cache. */
function onLockEvent({ locked: l, soft }: { locked: boolean; soft: boolean }) {
  if (soft) {
    // Soft wipe: identity not cached, but the overlay stays down and onLock does
    // NOT fire (a revealed secret / create draft must survive it). This is the
    // one case where `identityCached` and `locked` diverge.
    identityCached.value = false;
    return;
  }
  // Hard transition: setLocked drives `locked`, fires onLock, and mirrors
  // `identityCached` (a hard lock wipes the cache; an unlock restores it).
  setLocked(l);
  if (!l) {
    releaseAuthWaiters();
  }
}

/**
 * Flip the hard-lock state. A hard transition mirrors the cache: locked ⇒
 * identity not cached, unlocked ⇒ cached. When transitioning to locked, fire
 * every registered `onLock` clearer (each in its own try/catch so one failure
 * can't mask the others) *after* the ref has flipped, so the clear happens in
 * the same synchronous turn as the lock. Public so tests can drive the hard
 * path.
 */
function setLocked(v: boolean) {
  identityCached.value = !v;
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

/**
 * Run `op` once the identity is cached. If it isn't, raise the per-op auth
 * overlay (without firing `onLock`) and park until the backend reports an unlock,
 * then run `op` — re-checking first, since under Immediate an earlier queued op
 * may have soft-wiped the identity again. Concurrent callers share one auth
 * prompt (single-flight). Plaintext identities (`identityCached` always true) run
 * straight through.
 *
 * This is a gate, not a lease: `identityCached` is a snapshot at the `while`
 * check, and ops are NOT serialized — so under Immediate a rapid double-action
 * can let a second op's `invoke` reach the backend right after the first's
 * soft-wipe. That is safe because the backend re-checks identity authoritatively
 * (`Store::get` returns `IdentityEncrypted` against an empty cache): a racing op
 * fails with a benign error rather than ever serving a secret without auth. A
 * mutex/queue would smooth the UX but is not needed for correctness.
 *
 * Cancellation: if the user dismisses the per-op auth overlay (e.g. via Android
 * back while it is up — see `cancelAuth`), this rejects with
 * `{ code: "AUTH_CANCELLED" }`. Callers MUST swallow that via
 * `isAuthCancelled(e)` and show no error UI — the op never ran.
 */
async function runWithAuth<T>(op: () => Promise<T>): Promise<T> {
  while (!identityCached.value) {
    await ensureUnlocked();
  }
  return op();
}
export { runWithAuth };

/** If the identity isn't cached, show the auth overlay once and await the unlock. */
async function ensureUnlocked() {
  if (identityCached.value) return;
  if (!authPromise) {
    authPrompted.value = true;
    authPromise = new Promise<void>((resolve, reject) => {
      resolveAuth = resolve;
      rejectAuth = reject;
    });
  }
  await authPromise;
}

/** On unlock: drop the per-op overlay and release every parked caller. */
function releaseAuthWaiters() {
  authPrompted.value = false;
  authPromise = null;
  rejectAuth = null;
  const r = resolveAuth;
  resolveAuth = null;
  r?.();
}

/** Error code carried by the rejection `cancelAuth()` issues to parked callers. */
export const AUTH_CANCELLED = "AUTH_CANCELLED";

/** True if `e` is the cancellation a parked `runWithAuth` caller receives when the
 *  user dismisses the per-op auth overlay (e.g. via Android back). Callers should
 *  swallow it silently — the user cancelled; the op never ran. This is the one
 *  canonical check, so future `runWithAuth` callers don't each re-derive it. */
export function isAuthCancelled(e: unknown): boolean {
  return (e as { code?: unknown } | null | undefined)?.code === AUTH_CANCELLED;
}

/**
 * Dismiss the per-op auth overlay as cancelled: drop `authPrompted`, clear the
 * single-flight promise, and reject every parked `runWithAuth` caller with
 * `{ code: AUTH_CANCELLED }`. No-op when no per-op auth is in flight (e.g. a hard
 * lock, where `authPromise` is null), so it is safe to wire unconditionally as a
 * back-press handler — on a hard lock it leaves the overlay up (back consumed).
 */
function cancelAuth() {
  if (!authPromise) return;
  authPrompted.value = false;
  const reject = rejectAuth;
  authPromise = null;
  resolveAuth = null;
  rejectAuth = null;
  reject?.({ code: AUTH_CANCELLED });
}

/** Test-only: reset the module singleton between cases. */
export function __resetLockStateForTests() {
  initialized = false;
  locked.value = true;
  ready.value = false;
  identityCached.value = false;
  authPrompted.value = false;
  authPromise = null;
  resolveAuth = null;
  rejectAuth = null;
  listeners.clear();
  unlisten?.();
  unlisten = null;
}

/**
 * Test-only: put the app in the "unlocked, identity cached" state page tests
 * assume when they exercise copy/show/create without mounting `App.vue` (which
 * is what calls `init()` in production). Call from `beforeEach` after a reset.
 */
export function __unlockForTests() {
  initialized = true;
  locked.value = false;
  identityCached.value = true;
  ready.value = true;
  authPrompted.value = false;
}
