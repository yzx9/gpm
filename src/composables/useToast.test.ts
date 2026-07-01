// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { globalToast, useToast, __resetToastForTests } from "./useToast";

describe("useToast", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    __resetToastForTests();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("globalToast sets the message", () => {
    globalToast("hello");
    expect(useToast().toast.value).toBe("hello");
  });

  it("globalToast auto-clears after the timeout", () => {
    globalToast("hi");
    expect(useToast().toast.value).toBe("hi");
    vi.advanceTimersByTime(3000);
    expect(useToast().toast.value).toBe("");
  });

  it("globalToast replaces an earlier message and resets its timer", () => {
    globalToast("first");
    vi.advanceTimersByTime(2000);
    globalToast("second");
    // 2s past "first" would have cleared it at 3s — but "second" reset the timer.
    vi.advanceTimersByTime(2000);
    expect(useToast().toast.value).toBe("second");
    vi.advanceTimersByTime(1000);
    expect(useToast().toast.value).toBe("");
  });
});
