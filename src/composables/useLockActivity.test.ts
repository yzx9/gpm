// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import type { LockMode } from "@/api";
import { invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ref, type Ref } from "vue";

import { createLockActivity, type LockActivity } from "./useLockActivity";

/** Dispatch a document event of the given type (no payload needed — the handler
 *  reads refs, not the event). */
function dispatch(type: string): void {
  document.dispatchEvent(new Event(type));
}

describe("createLockActivity", () => {
  let lockMode: Ref<LockMode>;
  let identityCached: Ref<boolean>;
  let s: LockActivity;

  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
    lockMode = ref<LockMode>("immediate");
    identityCached = ref(true);
    s = createLockActivity(lockMode, identityCached, { throttleMs: 1000 });
    s.init();
  });

  afterEach(() => {
    s.dispose();
    vi.useRealTimers();
  });

  it("Idle + cached ⇒ pointerdown sends one bump", () => {
    lockMode.value = { idle: 300 };
    dispatch("pointerdown");
    expect(invoke).toHaveBeenCalledWith("bump_idle_timer");
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it("Immediate ⇒ no bump", () => {
    lockMode.value = "immediate";
    dispatch("pointerdown");
    expect(invoke).not.toHaveBeenCalledWith("bump_idle_timer");
  });

  it("Never ⇒ no bump", () => {
    lockMode.value = "never";
    dispatch("pointerdown");
    expect(invoke).not.toHaveBeenCalledWith("bump_idle_timer");
  });

  it("Idle but identity not cached ⇒ no bump", () => {
    lockMode.value = { idle: 300 };
    identityCached.value = false;
    dispatch("pointerdown");
    expect(invoke).not.toHaveBeenCalledWith("bump_idle_timer");
  });

  it("throttle: two pointerdowns within the window ⇒ one bump", () => {
    lockMode.value = { idle: 300 };
    dispatch("pointerdown");
    dispatch("pointerdown");
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it("throttle: after the window elapses, the next pointerdown bumps again", () => {
    lockMode.value = { idle: 300 };
    dispatch("pointerdown");
    vi.advanceTimersByTime(1000);
    dispatch("pointerdown");
    expect(invoke).toHaveBeenCalledTimes(2);
  });

  it("throttle is instance-wide: pointerdown then keydown within window ⇒ one bump", () => {
    lockMode.value = { idle: 300 };
    dispatch("pointerdown");
    dispatch("keydown");
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it("reactive mode flip: Immediate→Idle starts bumping, back to Immediate stops it", () => {
    // Start Immediate — no bump.
    dispatch("pointerdown");
    expect(invoke).not.toHaveBeenCalledWith("bump_idle_timer");
    // Flip to Idle (no re-init) — now activity bumps.
    lockMode.value = { idle: 300 };
    dispatch("pointerdown");
    expect(invoke).toHaveBeenCalledWith("bump_idle_timer");
    expect(invoke).toHaveBeenCalledTimes(1);
    // Past the throttle window, flip back to Immediate — bumping stops.
    vi.advanceTimersByTime(1000);
    lockMode.value = "immediate";
    dispatch("pointerdown");
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it("dispose() stops bumping", () => {
    lockMode.value = { idle: 300 };
    s.dispose();
    dispatch("pointerdown");
    expect(invoke).not.toHaveBeenCalledWith("bump_idle_timer");
  });

  it("init() is idempotent — a second init attaches no new listeners", () => {
    // The first init() ran in beforeEach. A spy installed now must see ZERO
    // addEventListener calls on a second init — proving the `initialized` guard
    // directly, not via throttle/event-dedup masking a double-attach.
    const addSpy = vi.spyOn(document, "addEventListener");
    s.init();
    expect(addSpy).not.toHaveBeenCalled();
  });

  it("default throttleMs (no opts) spaces bumps ~5000ms", () => {
    s.dispose(); // avoid the beforeEach instance double-counting dispatches
    const def = createLockActivity(lockMode, identityCached); // no opts ⇒ 5000
    def.init();
    lockMode.value = { idle: 300 };
    dispatch("pointerdown"); // first bump (last starts at -Infinity)
    expect(invoke).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(4999);
    dispatch("pointerdown"); // within 5000ms ⇒ dropped
    expect(invoke).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(2); // 5001ms total ⇒ window elapsed
    dispatch("pointerdown");
    expect(invoke).toHaveBeenCalledTimes(2);
    def.dispose();
  });

  it("every EVENT_TYPE triggers a bump (guards the registered event list)", () => {
    s.dispose();
    const all = createLockActivity(lockMode, identityCached, { throttleMs: 0 });
    all.init();
    lockMode.value = { idle: 300 };
    for (const t of ["pointerdown", "keydown", "wheel", "touchmove", "input"]) {
      dispatch(t);
    }
    expect(invoke).toHaveBeenCalledTimes(5);
    all.dispose();
  });
});
