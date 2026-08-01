// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { withSetup } from "@/test/withSetup";
import { afterEach, describe, expect, it, vi } from "vitest";
import { usePullToRefresh } from "./usePullToRefresh";

// jsdom has no TouchEvent constructor and no real touch layout. The composable
// only reads `e.touches[0].clientY` and calls `preventDefault` (cancelable), so a
// bare Event carrying a synthetic `touches` array is enough to drive every state
// transition deterministically.
function touch(type: string, clientY: number): Event {
  const ev = new Event(type, { cancelable: true, bubbles: true });
  Object.defineProperty(ev, "touches", {
    value: [{ clientY } as Touch],
    configurable: true,
  });
  return ev;
}

function fire(type: string, clientY: number): void {
  window.dispatchEvent(touch(type, clientY));
}

/** Override window.scrollY for one test (atTop() gate). Restored in afterEach. */
function setScrollY(value: number): void {
  Object.defineProperty(window, "scrollY", {
    value,
    configurable: true,
    writable: true,
  });
}

describe("usePullToRefresh", () => {
  const teardowns: (() => void)[] = [];

  afterEach(() => {
    teardowns.forEach((t) => t());
    teardowns.length = 0;
    setScrollY(0); // reset the atTop() gate between tests
  });

  function mount(opts: Parameters<typeof usePullToRefresh>[0]) {
    const [state, app] = withSetup(() => usePullToRefresh(opts));
    teardowns.push(() => app.unmount());
    return state;
  }

  it("arms past the threshold and fires onRefresh on release, then snaps back", () => {
    const onRefresh = vi.fn();
    const { pullDistance, armed } = mount({
      onRefresh,
      threshold: 70,
      damping: 0.5,
    });

    fire("touchstart", 100);
    fire("touchmove", 240); // delta 140 → damped 70 → armed
    expect(armed.value).toBe(true);
    expect(pullDistance.value).toBe(70);

    fire("touchend", 240);
    expect(onRefresh).toHaveBeenCalledTimes(1);
    expect(pullDistance.value).toBe(0);
    expect(armed.value).toBe(false);
  });

  it("does NOT fire onRefresh on a sub-threshold pull (snap-back only)", () => {
    const onRefresh = vi.fn();
    mount({ onRefresh, threshold: 70, damping: 0.5 });

    fire("touchstart", 100);
    fire("touchmove", 150); // delta 50 → damped 25 < 70
    fire("touchend", 150);
    expect(onRefresh).not.toHaveBeenCalled();
  });

  it("touchcancel resets the gesture without firing (no stuck indicator, no ghost refresh)", () => {
    const onRefresh = vi.fn();
    const { pullDistance, armed } = mount({
      onRefresh,
      threshold: 70,
      damping: 0.5,
    });

    fire("touchstart", 100);
    fire("touchmove", 240); // armed
    expect(armed.value).toBe(true);

    fire("touchcancel", 240); // OS / back-gesture took over
    expect(pullDistance.value).toBe(0);
    expect(armed.value).toBe(false);
    expect(onRefresh).not.toHaveBeenCalled();
  });

  it("enabled()=false suppresses activation entirely (modal / overlay gate)", () => {
    const onRefresh = vi.fn();
    const enabled = vi.fn(() => false);
    const { pullDistance, armed } = mount({
      onRefresh,
      enabled,
      threshold: 70,
      damping: 0.5,
    });

    fire("touchstart", 100);
    fire("touchmove", 240);
    fire("touchend", 240);

    expect(onRefresh).not.toHaveBeenCalled();
    expect(pullDistance.value).toBe(0);
    expect(armed.value).toBe(false);
  });

  it("enabled() flipping false mid-pull cancels the in-progress gesture", () => {
    const onRefresh = vi.fn();
    let gate = true;
    const enabled = () => gate;
    const { pullDistance, armed } = mount({
      onRefresh,
      enabled,
      threshold: 70,
      damping: 0.5,
    });

    fire("touchstart", 100);
    fire("touchmove", 240); // armed
    expect(armed.value).toBe(true);

    gate = false; // a modal opens mid-pull
    fire("touchmove", 250);
    expect(pullDistance.value).toBe(0); // cancelled immediately
    expect(armed.value).toBe(false);

    fire("touchend", 250);
    expect(onRefresh).not.toHaveBeenCalled();
  });

  it("ignores a pull that starts away from the top (scrollY > 0)", () => {
    const onRefresh = vi.fn();
    mount({ onRefresh, threshold: 70, damping: 0.5 });

    setScrollY(50);
    fire("touchstart", 100);
    fire("touchmove", 240);
    fire("touchend", 240);
    expect(onRefresh).not.toHaveBeenCalled();
  });

  it("an upward drag never engages (normal scroll-up stays native)", () => {
    const onRefresh = vi.fn();
    const { pullDistance } = mount({
      onRefresh,
      threshold: 70,
      damping: 0.5,
    });

    fire("touchstart", 100);
    fire("touchmove", 60); // delta -40 (upward) → no engage
    expect(pullDistance.value).toBe(0);
    fire("touchend", 60);
    expect(onRefresh).not.toHaveBeenCalled();
  });

  it("removes its window listeners on unmount (no leak / no fire after teardown)", () => {
    const onRefresh = vi.fn();
    const [, app] = withSetup(() =>
      usePullToRefresh({ onRefresh, threshold: 70, damping: 0.5 }),
    );
    app.unmount(); // fires onBeforeUnmount → listeners removed

    fire("touchstart", 100);
    fire("touchmove", 240);
    fire("touchend", 240);
    expect(onRefresh).not.toHaveBeenCalled();
  });

  it("ignores a second finger: only the initiating touch drives the pull", () => {
    const onRefresh = vi.fn();
    const { pullDistance, armed } = mount({
      onRefresh,
      threshold: 70,
      damping: 0.5,
    });

    // Dispatch a touch event carrying an explicit identifier list.
    const ev = (
      type: string,
      touches: { identifier?: number; clientY: number }[],
    ): void => {
      const e = new Event(type, { cancelable: true, bubbles: true });
      Object.defineProperty(e, "touches", {
        value: touches as Touch[],
        configurable: true,
      });
      window.dispatchEvent(e);
    };

    // Finger A (identifier 0) starts the gesture.
    ev("touchstart", [{ identifier: 0, clientY: 100 }]);
    // A move reporting only a different finger (identifier 1) is ignored — it
    // must not shift startY or arm the pull.
    ev("touchmove", [{ identifier: 1, clientY: 300 }]);
    expect(pullDistance.value).toBe(0);
    expect(armed.value).toBe(false);
    // The original finger still drives it.
    ev("touchmove", [{ identifier: 0, clientY: 240 }]);
    expect(armed.value).toBe(true);
  });

  it("engages at a sub-pixel scrollY (<= 1, not strict === 0)", () => {
    const onRefresh = vi.fn();
    setScrollY(0.5); // WebView can leave a fractional scrollY after scroll-restore
    const { armed } = mount({ onRefresh, threshold: 70, damping: 0.5 });

    fire("touchstart", 100);
    fire("touchmove", 240);
    expect(armed.value).toBe(true);
  });

  it("does not re-fire onRefresh while the refresh handler is still in flight (single-flight)", () => {
    // Mirrors the page wiring: enabled() reads a `pulling` flag that onRefresh
    // sets and clears on completion. A second gesture while that flag is true
    // must be suppressed so the consumer (syncRepo) isn't re-entered.
    let inFlight = false;
    const onRefresh = vi.fn(() => {
      inFlight = true; // stand-in for the page's `pulling.value = true`
    });

    mount({ onRefresh, enabled: () => !inFlight, threshold: 70, damping: 0.5 });

    fire("touchstart", 100);
    fire("touchmove", 240); // arm
    fire("touchend", 240); // release → onRefresh fires once
    expect(onRefresh).toHaveBeenCalledTimes(1);

    // A second gesture while the handler is still in flight is gated.
    fire("touchstart", 100);
    fire("touchmove", 240);
    fire("touchend", 240);
    expect(onRefresh).toHaveBeenCalledTimes(1);
  });
});
