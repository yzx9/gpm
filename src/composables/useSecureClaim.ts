// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { onScopeDispose, ref, type Ref } from "vue";
import { useSecureScreen } from "./useSecureScreen";

/**
 * Component-level screen-capture-protection primitive (R031). Each secret-bearing
 * component acquires a claim before its secret renders and releases it once the
 * secret is gone; the app-wide `useSecureScreen` ORs every live claim into the
 * single window-level `FLAG_SECURE`.
 *
 * The contract this enforces (the R031 invariant):
 *  - **Raise before render:** `withClaim`/`acquire` await the `set_secure` IPC
 *    before returning, and the result is branded `Claimed<T>`. Secret-holding
 *    refs and `useSecretReveal.reveal` accept ONLY `Claimed<T>`, so a component
 *    that fetches a secret without going through the claim fails `vue-tsc` — a
 *    compile-time fail-closed guard (no silent unprotected render).
 *  - **Clear before drop:** content is wiped in `onBeforeUnmount` (via the
 *    page's `useWipeOnLeave`); the claim is released here in `onScopeDispose`,
 *    which Vue runs AFTER `onBeforeUnmount`. So the secret is gone from the DOM
 *    before FLAG_SECURE can drop — by lifecycle ordering, independent of
 *    `<Transition>` timing.
 *
 * Must be called during a component's `setup()` (uses `onScopeDispose`).
 */

/** Opaque brand marking a value produced under an active secure claim. Only
 *  `withClaim` can create one (it acquires FLAG_SECURE before the value arrives).
 *  The brand is compile-time only — at runtime a `Claimed<T>` is just a `T`. */
declare const __claimBrand: unique symbol;
export type Claimed<T> = T & { readonly [__claimBrand]: true };

/** A per-component handle on the app-wide claim counter. */
export interface SecureClaim {
  /** Acquire one claim (raises FLAG_SECURE). Returns false if the IPC failed —
   *  the caller must not render the secret. */
  acquire: () => Promise<boolean>;
  /** Release one claim (idempotent; a no-op once the scope has released all it
   *  holds). Synchronous: the IPC runs after the caller's wipe, so content is
   *  cleared before FLAG_SECURE drops (see file doc). */
  release: () => void;
  /** Acquire a claim, run `op`, brand its result. Releases and rethrows if `op`
   *  throws (so an auth-cancel / decrypt failure doesn't strand the claim).
   *  Returns `null` if the acquire failed — the caller must not render. */
  withClaim: <T>(op: () => Promise<T>) => Promise<Claimed<T> | null>;
  /** `true` once an acquire on this scope has succeeded. Persistent pages gate
   *  their secret rendering on this (so nothing secret paints before the flag
   *  is confirmed up). */
  readonly ready: Ref<boolean>;
}

/**
 * Acquire/release a screen-capture-protection claim scoped to the calling
 * component. Any claim still held at unmount is released via `onScopeDispose`.
 */
export function useSecureClaim(): SecureClaim {
  const { acquireClaim, releaseClaim } = useSecureScreen();
  /** Claims this component currently holds (acquired minus released). */
  let held = 0;
  const ready = ref(false);

  async function acquire(): Promise<boolean> {
    const ok = await acquireClaim();
    if (ok) {
      held++;
      ready.value = true;
    }
    return ok;
  }

  function release(): void {
    if (held > 0) {
      held--;
      void releaseClaim();
    }
  }

  // Release everything this scope still holds when it unmounts — a forgotten
  // release can't strand FLAG_SECURE on. Vue runs this AFTER onBeforeUnmount,
  // so a page that wipes its secret in onBeforeUnmount has already cleared the
  // content before this drops the flag.
  onScopeDispose(() => {
    while (held > 0) release();
  });

  async function withClaim<T>(
    op: () => Promise<T>,
  ): Promise<Claimed<T> | null> {
    const ok = await acquire();
    if (!ok) return null;
    try {
      return (await op()) as Claimed<T>;
    } catch (e) {
      // op threw (auth-cancelled, decrypt failed) — don't strand the claim.
      release();
      throw e;
    }
  }

  return { acquire, release, withClaim, ready };
}
