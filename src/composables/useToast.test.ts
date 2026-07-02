// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { createToast, type ToastState } from "./useToast";

describe("useToast", () => {
  let t: ToastState;

  beforeEach(() => {
    vi.useFakeTimers();
    // Fresh per test — replaces the old module-singleton __resetToastForTests.
    t = createToast();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("globalToast sets the message", () => {
    t.globalToast("hello");
    expect(t.toast.value).toBe("hello");
  });

  it("globalToast auto-clears after the timeout", () => {
    t.globalToast("hi");
    expect(t.toast.value).toBe("hi");
    vi.advanceTimersByTime(3000);
    expect(t.toast.value).toBe("");
  });

  it("globalToast replaces an earlier message and resets its timer", () => {
    t.globalToast("first");
    vi.advanceTimersByTime(2000);
    t.globalToast("second");
    // 2s past "first" would have cleared it at 3s — but "second" reset the timer.
    vi.advanceTimersByTime(2000);
    expect(t.toast.value).toBe("second");
    vi.advanceTimersByTime(1000);
    expect(t.toast.value).toBe("");
  });
});
