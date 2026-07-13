// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { throttle } from "./throttle";

describe("throttle", () => {
  beforeEach(() => {
    // Fake timers + epoch 0 prove the `-Infinity` initial fires the first call
    // even when Date.now() is 0 (where a naive `last = 0` would drop it).
    vi.useFakeTimers();
    vi.setSystemTime(0);
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("invokes on the first call (even at clock epoch 0)", () => {
    const fn = vi.fn();
    const t = throttle(fn, 1000);
    t();
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it("drops calls within the interval", () => {
    const fn = vi.fn();
    const t = throttle(fn, 1000);
    t();
    vi.advanceTimersByTime(500);
    t();
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it("invokes again once the interval has elapsed", () => {
    const fn = vi.fn();
    const t = throttle(fn, 1000);
    t();
    vi.advanceTimersByTime(1000);
    t();
    expect(fn).toHaveBeenCalledTimes(2);
  });

  it("forwards arguments to the wrapped fn", () => {
    const fn = vi.fn();
    const t = throttle(fn, 1000);
    t("a", 1, true);
    expect(fn).toHaveBeenCalledWith("a", 1, true);
  });

  it("with intervalMs 0, invokes on every call", () => {
    const fn = vi.fn();
    const t = throttle(fn, 0);
    t();
    t();
    t();
    expect(fn).toHaveBeenCalledTimes(3);
  });
});
