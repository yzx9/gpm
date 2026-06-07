// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect } from "vitest";
import { formatRelativeTime } from "./format";

describe("formatRelativeTime", () => {
  const now = 100_000_000;

  it("returns 'just now' for less than 60 seconds", () => {
    expect(formatRelativeTime(now, now)).toBe("just now");
    expect(formatRelativeTime(now, now - 59_000)).toBe("just now");
    expect(formatRelativeTime(now, now - 1)).toBe("just now");
  });

  it("returns '1m ago' for 60–119 seconds", () => {
    expect(formatRelativeTime(now, now - 60_000)).toBe("1m ago");
    expect(formatRelativeTime(now, now - 119_000)).toBe("1m ago");
  });

  it("returns minute-based strings", () => {
    expect(formatRelativeTime(now, now - 120_000)).toBe("2m ago");
    expect(formatRelativeTime(now, now - 1800_000)).toBe("30m ago");
    expect(formatRelativeTime(now, now - 3540_000)).toBe("59m ago");
  });

  it("returns '1h ago' for 3600 seconds", () => {
    expect(formatRelativeTime(now, now - 3600_000)).toBe("1h ago");
  });

  it("returns hour-based strings", () => {
    expect(formatRelativeTime(now, now - 7200_000)).toBe("2h ago");
    expect(formatRelativeTime(now, now - 36000_000)).toBe("10h ago");
  });

  it("handles boundary at exactly 60 seconds", () => {
    expect(formatRelativeTime(now, now - 60_000)).toBe("1m ago");
    expect(formatRelativeTime(now, now - 59_999)).toBe("just now");
  });

  it("handles boundary at exactly 3600 seconds (1 hour)", () => {
    expect(formatRelativeTime(now, now - 3600_000)).toBe("1h ago");
    expect(formatRelativeTime(now, now - 3599_000)).toBe("59m ago");
  });
});
