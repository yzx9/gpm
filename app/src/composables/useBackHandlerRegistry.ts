// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { subscribeBackButton, type PluginListener } from "@/api";
import type { InjectionKey } from "vue";

/**
 * App-wide registry of Android back-button handlers (R062). Replaces the old
 * per-instance `subscribeBackButton` listeners — Tauri broadcasts a back press
 * to EVERY listener, so two stacked overlays both fired. Here a single global
 * listener dispatches a press to ONE handler: the highest-z entry, ties broken
 * by most-recent push (LIFO). z is the explicit primary key (a gate beats any
 * overlay regardless of mount order); LIFO is only the within-tier tie-break.
 *
 * Structural template: the controller-with-injection idiom from `useScrollLock`
 * (`createScrollLockController` + a shared default + injectable for tests). The
 * LIFECYCLE differs — scroll-lock is mount-bound (`onMounted`/`onBeforeUnmount`),
 * this registry is driven by show/hide (`watch(shown)` push/pop) — so do not
 * claim it "mirrors useScrollLock" beyond the injection pattern.
 *
 * The registry owns the ONE `subscribeBackButton` listener: lazily registered
 * when the first handler is pushed, unregistered when the stack drains, so
 * Tauri's default back behavior (webview goBack / exit) is left untouched while
 * no overlay is up. Two async transitions are generation-guarded so a stale
 * listener can never survive: (a) empty→register that resolves after the stack
 * drains again, and (b) non-empty→unregister that resolves after a re-push.
 *
 * Android-only in effect: `onBackButtonPress` only emits on Android; on desktop
 * the single listener is idle.
 */

/** Opaque handle to a pushed entry; pass to {@link BackHandlerRegistry.pop}. */
declare const __handleBrand: unique symbol;
export type BackHandlerHandle = number & { readonly [__handleBrand]: true };

export interface BackHandlerRegistry {
  /** Register a handler at z tier `z`. Returns a handle for later `pop`. */
  push(onBack: () => void, z: number): BackHandlerHandle;
  /** Remove the entry by handle identity (position-independent). No-op if gone. */
  pop(handle: BackHandlerHandle): void;
}

interface Entry {
  seq: number;
  onBack: () => void;
  z: number;
}

export function createBackHandlerRegistry(): BackHandlerRegistry {
  let entries: Entry[] = [];
  let nextSeq = 0;
  let listener: PluginListener | null = null;
  // Guards the async register/unregister transitions. `subscribing` prevents a
  // second subscribe IPC racing a first in-flight one; `releasing` makes a push
  // during an in-flight unregister wait (release re-triggers subscribe after)
  // so two native listeners are never live at once. Both reset in `finally` so a
  // rejected IPC can't brick back app-wide.
  let subscribing = false;
  let releasing = false;

  /** The single global back-press callback: fire exactly one handler. */
  function dispatch(): void {
    if (entries.length === 0) return;
    // Highest z; tie → highest seq (most-recent push = LIFO).
    let top = entries[0]!;
    for (const e of entries) {
      if (e.z > top.z || (e.z === top.z && e.seq > top.seq)) top = e;
    }
    try {
      top.onBack();
    } catch (e) {
      // A throwing handler must not break back for the rest of the app. Leave
      // the entry — the overlay still needs dismissal; surfacing the error is
      // enough.
      console.error("[gpm:back-handler] dispatch threw", e);
    }
  }

  async function ensureListener(): Promise<void> {
    // Wait if a release is mid-unregister — subscribing now would race a second
    // listener live alongside the one being torn down (a double-fire).
    // releaseListener re-triggers ensureListener once it finishes.
    if (listener || subscribing || releasing) return;
    if (entries.length === 0) return; // nothing pushed → nothing to register for
    subscribing = true;
    try {
      const l = await subscribeBackButton(dispatch);
      // The stack may have drained during the IPC round-trip — drop the stale
      // listener so a single back press never fires into an empty stack.
      if (entries.length === 0) {
        void l.unregister();
      } else {
        listener = l;
      }
    } catch (e) {
      // A failed subscribe must not brick back app-wide: leave listener null and
      // let a later push retry via ensureListener.
      console.error("[gpm:back-handler] subscribe failed", e);
    } finally {
      subscribing = false;
    }
  }

  async function releaseListener(): Promise<void> {
    if (!listener) return;
    if (entries.length > 0) return; // still in use by a remaining entry
    const l = listener;
    listener = null;
    releasing = true;
    try {
      await l.unregister();
    } catch (e) {
      console.error("[gpm:back-handler] unsubscribe failed", e);
    } finally {
      releasing = false;
      // A push that arrived during the await early-returned in ensureListener
      // (on `releasing`); re-acquire now so there's never a window with no live
      // listener while entries exist.
      if (entries.length > 0) void ensureListener();
    }
  }

  return {
    push(onBack, z) {
      const seq = ++nextSeq;
      entries.push({ seq, onBack, z });
      void ensureListener();
      return seq as BackHandlerHandle;
    },
    pop(handle) {
      const seq = handle as number;
      const before = entries.length;
      entries = entries.filter((e) => e.seq !== seq);
      if (entries.length !== before) {
        void releaseListener();
      }
    },
  };
}

// One shared registry PER APP, provided via BACK_HANDLER_KEY (provide/inject —
// the same pattern as DIALOG_KEY/TOAST_KEY), never a module singleton. main.ts
// provides the production instance; mountWithApp + the bare-mount component
// tests provide a fresh instance each, so no shared mutable state leaks across
// tests.
export const BACK_HANDLER_KEY: InjectionKey<BackHandlerRegistry> = Symbol(
  "BackHandlerRegistry",
);
