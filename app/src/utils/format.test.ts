// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { formatRelativeTime } from "./format";

describe("formatRelativeTime", () => {
  const now = 100_000_000;

  it("returns 'just now' for less than 60 seconds (defaulting to English)", () => {
    expect(formatRelativeTime(now, now)).toBe("just now");
    expect(formatRelativeTime(now, now - 59_000)).toBe("just now");
    expect(formatRelativeTime(now, now - 1)).toBe("just now");
  });

  it("localizes the sub-minute bucket per locale", () => {
    expect(formatRelativeTime(now, now - 30_000, "zh-CN")).toBe("刚刚");
  });

  it("formats the minute bucket via Intl for each locale", () => {
    expect(formatRelativeTime(now, now - 60_000, "en")).toBe("1 minute ago");
    expect(formatRelativeTime(now, now - 120_000, "en")).toBe("2 minutes ago");
    expect(formatRelativeTime(now, now - 1800_000, "en")).toBe(
      "30 minutes ago",
    );
    expect(formatRelativeTime(now, now - 120_000, "zh-CN")).toBe("2分钟前");
    expect(formatRelativeTime(now, now - 1800_000, "zh-CN")).toBe("30分钟前");
  });

  it("formats the hour bucket via Intl for each locale", () => {
    expect(formatRelativeTime(now, now - 3600_000, "en")).toBe("1 hour ago");
    expect(formatRelativeTime(now, now - 7200_000, "en")).toBe("2 hours ago");
    expect(formatRelativeTime(now, now - 36000_000, "en")).toBe("10 hours ago");
    expect(formatRelativeTime(now, now - 7200_000, "zh-CN")).toBe("2小时前");
  });

  it("formats the day bucket via Intl for each locale", () => {
    expect(formatRelativeTime(now, now - 86_400_000, "en")).toBe("1 day ago");
    expect(formatRelativeTime(now, now - 5 * 86_400_000, "en")).toBe(
      "5 days ago",
    );
    expect(formatRelativeTime(now, now - 5 * 86_400_000, "zh-CN")).toBe(
      "5天前",
    );
  });

  it("handles the day/hour boundary at exactly 24 hours", () => {
    expect(formatRelativeTime(now, now - 86_400_000, "en")).toBe("1 day ago");
    expect(formatRelativeTime(now, now - 86_399_000, "en")).toBe(
      "23 hours ago",
    );
  });

  it("falls back to an absolute date past a week instead of a relative form", () => {
    // Regression guard: a 249h-old timestamp used to read "249h ago".
    const result = formatRelativeTime(now, now - 249 * 3_600_000, "en");
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
    expect(formatRelativeTime(sameYearNow, priorYearTs, "en")).toMatch(
      /^Mar \d{1,2}, 2024$/,
    );
    expect(formatRelativeTime(sameYearNow, priorYearTs, "zh-CN")).toMatch(
      /^2024年\d{1,2}月\d{1,2}日$/,
    );
  });

  it("omits the year for a same-year absolute date", () => {
    const sameYearNow = Date.UTC(2025, 5, 15, 12, 0, 0); // 2025-06-15
    const sameYearTs = Date.UTC(2025, 2, 15, 12, 0, 0); // 2025-03-15
    expect(formatRelativeTime(sameYearNow, sameYearTs, "en")).toMatch(
      /^Mar \d{1,2}$/,
    );
    expect(formatRelativeTime(sameYearNow, sameYearTs, "zh-CN")).toMatch(
      /^\d{1,2}月\d{1,2}日$/,
    );
  });

  it("handles boundary at exactly 60 seconds", () => {
    expect(formatRelativeTime(now, now - 60_000, "en")).toBe("1 minute ago");
    expect(formatRelativeTime(now, now - 59_999, "en")).toBe("just now");
  });

  it("handles boundary at exactly 3600 seconds (1 hour)", () => {
    expect(formatRelativeTime(now, now - 3600_000, "en")).toBe("1 hour ago");
    expect(formatRelativeTime(now, now - 3599_000, "en")).toBe(
      "59 minutes ago",
    );
  });
});
