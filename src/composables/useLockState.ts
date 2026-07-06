// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import {
  getAuthState,
  subscribeIdentityLockState,
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

/** Error code carried by the rejection `cancelAuth()` issues to parked callers. */
export const AUTH_CANCELLED = "AUTH_CANCELLED";

/** True if `e` is the cancellation a parked `runWithAuth` caller receives when the
 *  user dismisses the per-op auth overlay (e.g. via Android back). Callers should
 *  swallow it silently — the user cancelled; the op never ran. This is the one
 *  canonical check, so future `runWithAuth` callers don't each re-derive it.
 *  Pure: takes no state, safe to import bare. */
export function isAuthCancelled(e: unknown): boolean {
  return (e as { code?: unknown } | null | undefined)?.code === AUTH_CANCELLED;
}

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
 * reveal being authenticated for), suppressed only while a hard-lock overlay
 * the user dismissed stays hidden (`overlayDismissed`) — the identity stays
 * locked, but the underlying (secret-free) page is reachable.
 *
 * Provided app-wide via `LOCK_KEY` (see `main.ts`): one instance, one event
 * listener. Tests construct their own via `createLockState()` so they never
 * share or reset a module singleton.
 */

/** The reactive lock state consumed by the UI and the auth gate. */
export interface LockState {
  /** Hard-lock flag — fail-closed `true` until `init()` reconciles with the backend. */
  readonly locked: Readonly<Ref<boolean>>;
  /** Whether the unlock overlay should be shown (hard lock or per-op auth,
   *  unless a hard-lock overlay was dismissed and stays hidden). */
  readonly overlayUp: ComputedRef<boolean>;
  /** Whether the decrypted identity is in the backend cache (next op needs no auth). */
  readonly identityCached: Readonly<Ref<boolean>>;
  /** False until `init()` has reconciled with the backend. */
  readonly ready: Readonly<Ref<boolean>>;
  /** Reflect backend state and arm the single `identity-lock-state` listener. Idempotent. */
  init: () => Promise<void>;
  /** Flip the hard-lock state (mirrors the cache, fires `onLock` clearers on lock). */
  setLocked: (v: boolean) => void;
  /** Register a hard-lock callback. Auto-removed on scope dispose. @returns an unsubscribe. */
  onLock: (cb: () => void) => () => void;
  /** Run `op` once the identity is cached, else raise the per-op auth overlay and park. */
  runWithAuth: <T>(op: () => Promise<T>) => Promise<T>;
  /** Dismiss the per-op auth overlay as cancelled (rejects parked callers with AUTH_CANCELLED). */
  cancelAuth: () => void;
  /** Dismiss the unlock overlay: cancels a per-op auth, or hides a hard-lock
   *  overlay WITHOUT unlocking (identity stays locked; next secret op re-prompts). */
  dismissOverlay: () => void;
}

/** Seed options for `createLockState` (test/seed only; production passes none). */
export interface CreateLockStateOptions {
  /**
   * Start in the "unlocked, identity cached" state page tests assume when they
   * exercise copy/show/create without `App.vue`'s `init()` (the old
   * `__unlockForTests` precondition): `locked=false`, `identityCached=true`,
   * `ready=true`, `initialized=true`.
   */
  unlocked?: boolean;
}

/** Injection key for the app-wide lock state. */
export const LOCK_KEY: InjectionKey<LockState> = Symbol("LockState");

/**
 * Create a fresh lock-state instance with its own refs, listeners, and event
 * listener. Production calls this once in `main.ts` and provides it; tests call
 * it per-case for isolation (no module singleton to reset).
 */
export function createLockState(opts: CreateLockStateOptions = {}): LockState {
  const unlocked = opts.unlocked === true;
  // Fail-closed by default: `locked` starts true until `init()` reconciles with
  // the backend, so the overlay shows during the brief boot window before we
  // know the real state.
  const locked = ref(unlocked ? false : true);
  // False until `init()` has reconciled; the unlocked seed flips it (tests skip init).
  const ready = ref(unlocked);
  // Whether the decrypted identity is in the backend cache (next op needs no
  // auth). Plaintext identities report `true` (they decrypt straight from disk);
  // the unlocked seed mirrors that.
  const identityCached = ref(unlocked);
  // Overlay is up specifically for a per-operation auth (Immediate mode), not a
  // hard lock. Drives `overlayUp` without firing `onLock`.
  const authPrompted = ref(false);
  // True after the user dismissed a HARD-lock overlay (× / backdrop / back).
  // The identity stays locked — this only suppresses `overlayUp` until the next
  // secret op re-raises it as a per-op auth (ensureUnlocked resets it) or a
  // fresh hard lock re-shows it (setLocked(true) resets it). Per-op auth never
  // sets this; cancelAuth handles its own dismiss.
  const overlayDismissed = ref(false);

  /// The shared promise awaiting op callers park on while the per-op auth overlay
  /// is up. All concurrent callers await the same one (single-flight: one prompt,
  /// not N). Resolved when the backend reports an unlock.
  let authPromise: Promise<void> | null = null;
  let resolveAuth: (() => void) | null = null;
  // Reject fn paired with `authPromise` — invoked by `cancelAuth()` to abort every
  // parked caller when the user dismisses the per-op auth overlay (e.g. Android back).
  let rejectAuth: ((e: unknown) => void) | null = null;

  let initialized = unlocked;
  const listeners = new Set<() => void>();
  let unlisten: UnlistenFn | null = null;

  /** Whether the unlock overlay should be shown: a hard lock or per-op auth,
   *  unless the user dismissed a hard-lock overlay (× / backdrop / back) — the
   *  identity stays locked, but the overlay stays hidden until the next secret
   *  op (per-op auth) or a fresh hard lock re-shows it. */
  const overlayUp = computed(
    () => (locked.value || authPrompted.value) && !overlayDismissed.value,
  );

  /**
   * Register a callback to run whenever the identity becomes _hard_-locked (manual
   * / idle). Auto-removed when the owning component/scope is disposed. Safe to
   * call outside a scope (e.g. in tests) — the callback is then only removable via
   * the returned fn.
   * NOTE: a soft wipe (Immediate post-op) does NOT fire this — it must not clear a
   * revealed secret or a create-form draft.
   *
   * @returns an unsubscribe function
   */
  function onLock(cb: () => void): () => void {
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
   * overlay down. This instance never decides state on its own.
   */
  async function init() {
    if (initialized) return;
    initialized = true;

    // Own the one listener that mirrors the backend's lock-state snapshot.
    unlisten ??= await subscribeIdentityLockState(onLockEvent);

    try {
      const auth = await getAuthState();
      // Unencrypted identities are never "locked" — there is nothing to unlock.
      // setLocked mirrors identityCached: encrypted+locked ⇒ not cached,
      // otherwise cached (plaintext always reads straight from disk).
      setLocked(auth.encrypted && !auth.unlocked);
    } catch {
      // Couldn't read auth state — stay fail-closed.
      setLocked(true);
    }
    // init() is the boundary between the speculative boot state (locked=true,
    // ready=false) and the real backend state. A dismiss during that boot
    // window would strand `overlayDismissed=true`; clear it now that the real
    // state is known so a genuine hard lock shows its overlay.
    overlayDismissed.value = false;
    ready.value = true;
  }

  /** Backend lock-state event → the two refs. Soft wipes touch only the cache. */
  function onLockEvent({
    locked: l,
    soft,
  }: {
    locked: boolean;
    soft: boolean;
  }) {
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
      // A fresh hard lock re-shows the overlay even if the user had dismissed
      // a previous one.
      overlayDismissed.value = false;
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

  /** If the identity isn't cached, show the auth overlay once and await the unlock. */
  async function ensureUnlocked() {
    if (identityCached.value) return;
    if (!authPromise) {
      // A dismissed hard lock must re-show for this per-op auth.
      overlayDismissed.value = false;
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

  /**
   * Dismiss the unlock overlay (× / backdrop / back). A per-op auth in flight
   * delegates to `cancelAuth` (rejecting parked callers with AUTH_CANCELLED);
   * a hard lock hides the overlay WITHOUT unlocking — the identity stays
   * locked, secrets stay wiped, and the next secret op re-prompts via per-op
   * auth. No-op when neither is up, so it is safe to wire unconditionally as a
   * close / back-press handler.
   */
  function dismissOverlay() {
    if (authPromise) {
      cancelAuth();
      return;
    }
    if (locked.value) {
      overlayDismissed.value = true;
    }
  }

  return {
    locked,
    overlayUp,
    identityCached,
    ready,
    init,
    setLocked,
    onLock,
    runWithAuth,
    cancelAuth,
    dismissOverlay,
  };
}

/**
 * Inject the app-wide lock state. Must be called within a component `setup()`
 * under a tree that provided `LOCK_KEY` — `main.ts` does this once for the whole
 * app. Throws if missing, so a forgotten `provide` fails loudly instead of
 * silently degrading to a phantom singleton.
 */
export function useLockState(): LockState {
  const s = inject(LOCK_KEY);
  if (!s) {
    throw new Error("useLockState() requires LOCK_KEY to be provided");
  }
  return s;
}
