// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
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

  it("returns 'Xd ago' for 1–6 days", () => {
    expect(formatRelativeTime(now, now - 86_400_000)).toBe("1d ago");
    expect(formatRelativeTime(now, now - 5 * 86_400_000)).toBe("5d ago");
    expect(formatRelativeTime(now, now - 6 * 86_400_000)).toBe("6d ago");
  });

  it("handles boundary at exactly 24 hours (1 day)", () => {
    expect(formatRelativeTime(now, now - 86_400_000)).toBe("1d ago");
    expect(formatRelativeTime(now, now - 86_399_000)).toBe("23h ago");
  });

  it("falls back to an absolute date past a week instead of 'Xh ago'", () => {
    // Regression guard: a 249h-old timestamp used to read "249h ago".
    const result = formatRelativeTime(now, now - 249 * 3_600_000);
    expect(result).not.toMatch(/ago$/);
    expect(result).toMatch(
      /^(Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) \d{1,2}(, \d{4})?$/,
    );
  });

  it("includes the year once the timestamp is in a prior year", () => {
    const sameYearNow = Date.UTC(2025, 5, 15, 12, 0, 0); // 2025-06-15
    const priorYearTs = Date.UTC(2024, 2, 15, 12, 0, 0); // 2024-03-15
    // Mid-month, noon UTC: the local-calendar day can shift by ±1 across
    // timezones, but the month and year stay fixed, so anchor with regex.
    expect(formatRelativeTime(sameYearNow, priorYearTs)).toMatch(
      /^Mar \d{1,2}, 2024$/,
    );
  });

  it("omits the year for a same-year absolute date", () => {
    const sameYearNow = Date.UTC(2025, 5, 15, 12, 0, 0); // 2025-06-15
    const sameYearTs = Date.UTC(2025, 2, 15, 12, 0, 0); // 2025-03-15
    expect(formatRelativeTime(sameYearNow, sameYearTs)).toMatch(
      /^Mar \d{1,2}$/,
    );
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
