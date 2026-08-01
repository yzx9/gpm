// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { onBeforeUnmount, onMounted, ref } from "vue";

/**
 * Pull-to-refresh for the entries list.
 *
 * Why Touch Events, not Pointer Events: PTR has to coexist with native list
 * scrolling — it must engage *only* for a downward drag pinned to the top of
 * the list and stay out of the way for every other gesture. That per-gesture,
 * scroll-position-aware interception is exactly what Touch Events'
 * `touchmove.preventDefault()` gives you; Pointer Events would need
 * `touch-action: none` on the scroll container to reliably overtake the
 * gesture, which kills native scrolling. The page already suppresses the
 * browser's own pull-to-refresh / rubber-band via `overscroll-behavior-y:
 * contain` on `html, body` (see `main.css`), so this composable owns the
 * gesture end-to-end. Desktop mouse is not a target (desktop has no touch
 * stream); desktop sync is handled out of band.
 *
 * State machine: idle → (touchstart at scrollY 0) → armed-candidate →
 * (downward touchmove) → pulling (indicator visible) → touchend (fires
 * `onRefresh` only if the damped pull crossed `threshold`) → idle. A
 * `touchcancel` at any point (Android back-gesture edge swipe, notification
 * shade drag, OS interruption) resets hard — no stuck indicator, no ghost
 * refresh off a gesture the user didn't finish.
 *
 * `enabled()` gates the whole thing: when it returns false (a divergence /
 * signature-block / signature-audit modal or the identity unlock overlay is
 * up) the composable is inert — no indicator, no refresh, and a pull already
 * in progress cancels immediately. This keeps a stray pull from racing an
 * open resolve flow (it would overwrite the `remote_tip` the user is
 * mid-decision on).
 *
 * Must be called from a component `setup()` (uses `onMounted`/`onBeforeUnmount`).
 */
export interface UsePullToRefreshOptions {
  /** Fired on a pull that crossed the threshold. Wire to `syncRepo`. */
  onRefresh: () => void;
  /** When false, PTR is fully inert (no indicator, no refresh). Default: always on. */
  enabled?: () => boolean;
  /** Damped pull distance (px) past which a release fires `onRefresh`. Default 70. */
  threshold?: number;
  /** 0..1 damping applied to the raw pull delta (rubber-band feel). Default 0.5. */
  damping?: number;
}

export function usePullToRefresh(opts: UsePullToRefreshOptions) {
  const threshold = opts.threshold ?? 70;
  const damping = opts.damping ?? 0.5;
  const enabled = opts.enabled ?? (() => true);

  /** Damped pull distance in px (drives the indicator's translate). 0 = hidden. */
  const pullDistance = ref(0);
  /** True once the pull crosses the threshold — release will fire `onRefresh`. */
  const armed = ref(false);

  // Gesture-local state — not reactive; only the indicator needs to re-render.
  let startY: number | null = null; // clientY of the touchstart that may become a pull
  // Which finger started the gesture. A second finger landing must not shift
  // `startY` (multi-touch), so move/end only follow the initiating touch.
  let startIdentifier: number = -1;
  let pulling = false; // a pull is in progress (indicator visible, touchmove preventDefaulted)

  function atTop(): boolean {
    // <= 1, not === 0: Android WebView can leave a sub-pixel scrollY after a
    // programmatic scrollTo or router scroll-restore; a strict-0 check would
    // silently disable PTR in that state.
    return typeof window !== "undefined" && window.scrollY <= 1;
  }

  function findTouch(list: TouchList, identifier: number): Touch | null {
    for (let i = 0; i < list.length; i++) {
      const t = list[i]!;
      if (t.identifier === identifier) return t;
    }
    return null;
  }

  function reset(): void {
    startY = null;
    startIdentifier = -1;
    pulling = false;
    pullDistance.value = 0;
    armed.value = false;
  }

  function onTouchStart(e: TouchEvent): void {
    if (!enabled() || !atTop()) return;
    const t = e.touches[0];
    if (!t) return;
    // Record the start but don't engage yet — wait for a downward move so an
    // upward swipe scrolls the list normally without showing the indicator.
    startY = t.clientY;
    startIdentifier = t.identifier;
    pulling = false;
    armed.value = false;
  }

  function onTouchMove(e: TouchEvent): void {
    if (startY === null) return;
    const t = findTouch(e.touches, startIdentifier);
    if (!t) return; // the initiating finger lifted or isn't driving this move
    // A modal/overlay appeared mid-pull — cancel the gesture immediately.
    if (!enabled()) {
      reset();
      return;
    }
    const delta = t.clientY - startY;
    if (delta <= 0) {
      // Dragging up = normal scroll; release our hold without firing.
      if (pulling) reset();
      return;
    }
    // Engage the pull (only meaningful while still pinned to the top).
    if (!pulling) {
      if (!atTop()) {
        startY = null;
        return;
      }
      pulling = true;
    }
    // Overtake the gesture: stop the page from also scrolling/rubber-banding.
    e.preventDefault();
    const damped = delta * damping;
    pullDistance.value = damped;
    armed.value = damped >= threshold;
  }

  function onTouchEnd(): void {
    if (pulling && armed.value && enabled()) {
      opts.onRefresh();
    }
    reset();
  }

  function onTouchCancel(): void {
    // OS/back-gesture took over the touch — never fire, never strand the indicator.
    reset();
  }

  onMounted(() => {
    // touchstart/end/cancel are passive observers; touchmove must be cancelable
    // so preventDefault can overtake the scroll only during an active pull.
    window.addEventListener("touchstart", onTouchStart, { passive: true });
    window.addEventListener("touchmove", onTouchMove, { passive: false });
    window.addEventListener("touchend", onTouchEnd, { passive: true });
    window.addEventListener("touchcancel", onTouchCancel, { passive: true });
  });

  onBeforeUnmount(() => {
    window.removeEventListener("touchstart", onTouchStart);
    window.removeEventListener("touchmove", onTouchMove);
    window.removeEventListener("touchend", onTouchEnd);
    window.removeEventListener("touchcancel", onTouchCancel);
  });

  return { pullDistance, armed };
}
