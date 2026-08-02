// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { SUPPORTED_LOCALES } from "@/i18n";
import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import { defineComponent, h } from "vue";
import { useI18n } from "vue-i18n";
import { formatRelativeTime, useRelativeTime } from "./useRelativeTime";

// The pure formatter is locale + justNow parameterized, so the sub-minute label
// is supplied directly here — no mount, no bundle load. zh-CN passes "刚刚";
// `justNow` is only read by the sub-minute branch, so it is irrelevant (but
// still passed) for the minute/hour/day/absolute assertions.
const JUST_NOW_EN = "just now";
const JUST_NOW_ZH = "刚刚";

describe("formatRelativeTime (pure)", () => {
  const now = 100_000_000;

  it("returns the caller-supplied justNow for less than 60 seconds", () => {
    expect(formatRelativeTime(now, now, "en", JUST_NOW_EN)).toBe("just now");
    expect(formatRelativeTime(now, now - 59_000, "en", JUST_NOW_EN)).toBe(
      "just now",
    );
    expect(formatRelativeTime(now, now - 1, "en", JUST_NOW_EN)).toBe(
      "just now",
    );
    expect(formatRelativeTime(now, now - 30_000, "zh-CN", JUST_NOW_ZH)).toBe(
      "刚刚",
    );
  });

  it("formats the minute bucket via Intl for each locale", () => {
    expect(formatRelativeTime(now, now - 60_000, "en", JUST_NOW_EN)).toBe(
      "1 minute ago",
    );
    expect(formatRelativeTime(now, now - 120_000, "en", JUST_NOW_EN)).toBe(
      "2 minutes ago",
    );
    expect(formatRelativeTime(now, now - 1_800_000, "en", JUST_NOW_EN)).toBe(
      "30 minutes ago",
    );
    expect(formatRelativeTime(now, now - 120_000, "zh-CN", JUST_NOW_ZH)).toBe(
      "2分钟前",
    );
    expect(formatRelativeTime(now, now - 1_800_000, "zh-CN", JUST_NOW_ZH)).toBe(
      "30分钟前",
    );
  });

  it("formats the hour bucket via Intl for each locale", () => {
    expect(formatRelativeTime(now, now - 3_600_000, "en", JUST_NOW_EN)).toBe(
      "1 hour ago",
    );
    expect(formatRelativeTime(now, now - 7_200_000, "en", JUST_NOW_EN)).toBe(
      "2 hours ago",
    );
    expect(formatRelativeTime(now, now - 36_000_000, "en", JUST_NOW_EN)).toBe(
      "10 hours ago",
    );
    expect(formatRelativeTime(now, now - 7_200_000, "zh-CN", JUST_NOW_ZH)).toBe(
      "2小时前",
    );
  });

  it("formats the day bucket via Intl for each locale", () => {
    expect(formatRelativeTime(now, now - 86_400_000, "en", JUST_NOW_EN)).toBe(
      "1 day ago",
    );
    expect(
      formatRelativeTime(now, now - 5 * 86_400_000, "en", JUST_NOW_EN),
    ).toBe("5 days ago");
    expect(
      formatRelativeTime(now, now - 5 * 86_400_000, "zh-CN", JUST_NOW_ZH),
    ).toBe("5天前");
  });

  it("handles the day/hour boundary at exactly 24 hours", () => {
    expect(formatRelativeTime(now, now - 86_400_000, "en", JUST_NOW_EN)).toBe(
      "1 day ago",
    );
    expect(formatRelativeTime(now, now - 86_399_000, "en", JUST_NOW_EN)).toBe(
      "23 hours ago",
    );
  });

  it("handles the day/absolute boundary at exactly 7 days", () => {
    // 1ms under 7 days stays in the relative day bucket.
    expect(
      formatRelativeTime(now, now - (7 * 86_400_000 - 1), "en", JUST_NOW_EN),
    ).toBe("6 days ago");
    // Exactly 7 days falls back to an absolute date.
    const result = formatRelativeTime(
      now,
      now - 7 * 86_400_000,
      "en",
      JUST_NOW_EN,
    );
    expect(result).not.toMatch(/ago$/);
    expect(result).toMatch(
      /^(Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) \d{1,2}(, \d{4})?$/,
    );
  });

  it("returns justNow for a future timestamp (clock skew)", () => {
    // A timestamp ahead of `now` makes `seconds` negative, which `seconds < 60`
    // collapses to the sub-minute bucket. For sync/commit labels that is the
    // friendly call (you can't show "in 2 hours" for a last-sync stamp); this
    // pins the behavior so a future change is visible.
    expect(formatRelativeTime(now, now + 5_000, "en", JUST_NOW_EN)).toBe(
      "just now",
    );
    expect(formatRelativeTime(now, now + 7_200_000, "en", JUST_NOW_EN)).toBe(
      "just now",
    );
  });

  it("falls back to an absolute date past a week instead of a relative form", () => {
    // Regression guard: a 249h-old timestamp used to read "249h ago".
    const result = formatRelativeTime(
      now,
      now - 249 * 3_600_000,
      "en",
      JUST_NOW_EN,
    );
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
    expect(
      formatRelativeTime(sameYearNow, priorYearTs, "en", JUST_NOW_EN),
    ).toMatch(/^Mar \d{1,2}, 2024$/);
    expect(
      formatRelativeTime(sameYearNow, priorYearTs, "zh-CN", JUST_NOW_ZH),
    ).toMatch(/^2024年\d{1,2}月\d{1,2}日$/);
  });

  it("omits the year for a same-year absolute date", () => {
    const sameYearNow = Date.UTC(2025, 5, 15, 12, 0, 0); // 2025-06-15
    const sameYearTs = Date.UTC(2025, 2, 15, 12, 0, 0); // 2025-03-15
    expect(
      formatRelativeTime(sameYearNow, sameYearTs, "en", JUST_NOW_EN),
    ).toMatch(/^Mar \d{1,2}$/);
    expect(
      formatRelativeTime(sameYearNow, sameYearTs, "zh-CN", JUST_NOW_ZH),
    ).toMatch(/^\d{1,2}月\d{1,2}日$/);
  });

  it("handles boundary at exactly 60 seconds", () => {
    expect(formatRelativeTime(now, now - 60_000, "en", JUST_NOW_EN)).toBe(
      "1 minute ago",
    );
    expect(formatRelativeTime(now, now - 59_999, "en", JUST_NOW_EN)).toBe(
      "just now",
    );
  });

  it("handles boundary at exactly 3600 seconds (1 hour)", () => {
    expect(formatRelativeTime(now, now - 3_600_000, "en", JUST_NOW_EN)).toBe(
      "1 hour ago",
    );
    expect(formatRelativeTime(now, now - 3_599_000, "en", JUST_NOW_EN)).toBe(
      "59 minutes ago",
    );
  });
});

describe("useRelativeTime", () => {
  it("binds the active locale and re-binds on a locale switch", () => {
    // Capture the locale ref the wrapper reads by pulling it from the same
    // `useI18n()` the host calls, so toggling it re-binds the formatter
    // regardless of which i18n instance the test harness installed.
    const ctx: {
      fmt?: (now: number, timestamp: number) => string;
      locale?: { value: string };
    } = {};
    const Host = defineComponent({
      setup() {
        ctx.locale = useI18n().locale;
        ctx.fmt = useRelativeTime().formatRelativeTime;
        return () => h("div");
      },
    });
    mount(Host);

    const now = 100_000_000;
    // en (default test locale): sub-minute label comes from the en common
    // bundle, and the minute bucket is Intl-driven.
    expect(ctx.fmt!(now, now)).toBe("just now");
    expect(ctx.fmt!(now, now - 60_000)).toBe("1 minute ago");
    // Switch to zh-CN: the Intl minute form re-binds without re-creating the
    // wrapper — this is the locale-tracking contract the composable provides.
    ctx.locale!.value = "zh-CN";
    expect(ctx.fmt!(now, now - 120_000)).toBe("2分钟前");
    // And back to en.
    ctx.locale!.value = "en";
    expect(ctx.fmt!(now, now - 60_000)).toBe("1 minute ago");
  });
});

describe("common.relativeTime.justNow bundle parity", () => {
  // Guards against a silent English fallback: a locale added to
  // SUPPORTED_LOCALES without a matching justNow key would render "just now"
  // for everyone via fallbackLocale. Reads each locale's OWN bundle directly
  // (not via the i18n instance) so a missing/mistyped key can't hide behind
  // fallbackLocale the way an instance `tm(key)` under the active locale would.
  it("every supported locale ships a non-empty justNow label in its bundle", async () => {
    for (const locale of SUPPORTED_LOCALES) {
      const mod = (await import(`@/locales/${locale}/common.json`)).default as {
        relativeTime?: { justNow?: string };
      };
      expect(
        mod.relativeTime?.justNow,
        `${locale} common.relativeTime.justNow`,
      ).toBeTruthy();
    }
  });
});
