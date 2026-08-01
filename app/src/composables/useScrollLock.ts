// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { onBeforeUnmount, onMounted } from "vue";

/**
 * Lock background scrolling while a modal/overlay is up.
 *
 * Why this is needed: `BaseModalShell`'s backdrop is a `position: fixed;
 * inset: 0` layer with a semi-transparent fill — it covers the viewport and
 * intercepts clicks, but it is NOT itself a scroll container. On a touch
 * WebView a drag that starts on a non-scrolling fixed element still chains to
 * the nearest scrollable ancestor, which here is the document scroller
 * (`html`/`body`). The app deliberately keeps window scroll on the document —
 * `style.css` uses `overflow-x: clip` (not `hidden`) on `.app-shell` precisely
 * so vertical scroll is NOT reparented into the shell — so that ancestor is
 * `documentElement`, and a drag on the backdrop scrolls the list behind the
 * dialog. `overscroll-behavior: contain` on the backdrop is a no-op here (it
 * only governs the boundary of a scroll container, and the backdrop isn't one
 * — see the comment in `style.css`). Locking the document scroller is the fix.
 *
 * Why `overflow: hidden` on `documentElement` (and not `touch-action: none` on
 * the backdrop, or locking `body`): setting `touch-action: none` on the
 * backdrop would also disable panning on the backdrop's descendants — the
 * `touch-action` a touch sees is the intersection of the touched element and
 * its ancestors, so an inner scroll region (`.div-scroll`, the divergence
 * modal's lists) could no longer scroll. `overflow: hidden` on the root
 * element freezes only the viewport scroller; descendants with their own
 * `overflow: auto` keep scrolling independently. Per CSS overflow propagation,
 * when the root element's overflow is non-`visible` the viewport uses the root
 * (the body stops propagating), so this is the element whose lock freezes the
 * window. Chromium preserves `scrollTop` across the toggle, so the page does
 * not jump to the top while locked and resumes in place on unlock — no
 * position bookkeeping needed (the `position: fixed` body trick is the one that
 * resets scroll and needs bookkeeping; this avoids it).
 *
 * Ref-counted: two shells can be up at once (e.g. a page modal — divergence /
 * block / audit — with the identity `UnlockModal` stacked above it via
 * `runWithAuth`). Each mount acquires and each unmount releases; the document
 * unlocks only when the last shell goes down, so an inner modal dismissing
 * never unlocks the page behind an outer one still showing.
 *
 * The counter lives on a controller instance so tests can construct a fresh
 * one per case (no module-singleton `__reset` hazard); `useScrollLock()` uses
 * a shared default controller so every shell in the app shares one count.
 *
 * Must be called from a component `setup()` (uses `onMounted`/`onBeforeUnmount`,
 * not `onActivated`/`onDeactivated` — a `<KeepAlive>` host would hold the lock
 * across deactivation and never re-pair on activation, so shells must not live
 * under a keep-alive scope).
 */
export interface ScrollLockController {
  /** Increment the lock count; freezes the document scroller on 0→1. */
  acquire: () => void;
  /** Decrement the lock count; unfreezes on last release. No-op at 0. */
  release: () => void;
}

export function createScrollLockController(): ScrollLockController {
  let count = 0;
  let savedOverflow = "";

  return {
    acquire() {
      if (count === 0) {
        const el = document.documentElement;
        savedOverflow = el.style.overflow;
        el.style.overflow = "hidden";
      }
      count++;
    },
    release() {
      // Guard a stray release with no matching acquire (defensive; the
      // composable pairs them, but a controller can be driven directly).
      if (count === 0) return;
      count--;
      if (count === 0) {
        document.documentElement.style.overflow = savedOverflow;
        savedOverflow = "";
      }
    },
  };
}

// One shared counter for the whole app — every BaseModalShell increments /
// decrements the same count so stacked shells keep the lock until the last one
// unmounts.
const defaultController = createScrollLockController();

/**
 * Lock the document scroller for the lifetime of the calling component. Pairs
 * `onMounted`/`onBeforeUnmount`, so a `v-if`-mounted shell locks on show and
 * unlocks on hide. Pass a controller (e.g. a fresh `createScrollLockController`
 * or a fake) to test in isolation from the app-wide counter.
 */
export function useScrollLock(
  controller: ScrollLockController = defaultController,
) {
  onMounted(() => controller.acquire());
  onBeforeUnmount(() => controller.release());
}
