// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

/**
 * Sub-minute label per locale. `Intl.RelativeTimeFormat` has no "just now"
 * concept (and renders awkward sub-second forms near zero), so the under-a-minute
 * bucket stays a short localized constant. Unknown locales fall back to English.
 */
const JUST_NOW: Record<string, string> = {
  en: "just now",
  "zh-CN": "刚刚",
};

// Entry lists render this per row, so cache the collators per locale rather than
// reallocating on every call. The relative formatter depends only on the locale;
// the absolute-date formatter also depends on whether the year is shown, so it is
// keyed by `locale|withYear`.
const rtfCache = new Map<string, Intl.RelativeTimeFormat>();
const dtfCache = new Map<string, Intl.DateTimeFormat>();

function relativeFormatter(locale: string): Intl.RelativeTimeFormat {
  let f = rtfCache.get(locale);
  if (!f) {
    // `numeric: "always"` keeps singular units literal ("1 minute ago") rather
    // than the "last minute"/"yesterday" idioms `auto` would substitute.
    f = new Intl.RelativeTimeFormat(locale, { numeric: "always" });
    rtfCache.set(locale, f);
  }
  return f;
}

function absoluteFormatter(
  locale: string,
  withYear: boolean,
): Intl.DateTimeFormat {
  const key = `${locale}|${withYear}`;
  let f = dtfCache.get(key);
  if (!f) {
    f = new Intl.DateTimeFormat(locale, {
      month: "short",
      day: "numeric",
      ...(withYear ? { year: "numeric" } : {}),
    });
    dtfCache.set(key, f);
  }
  return f;
}

/**
 * Format a timestamp as a compact, human-readable time label.
 *
 * Recent timestamps use a relative form ("just now", "5 minutes ago", "3 hours
 * ago", "2 days ago"). Past a week the relative form stops carrying meaning —
 * "249 hours ago" requires arithmetic to parse — so we fall back to an absolute
 * calendar date (e.g. "Mar 15", or "Mar 15, 2024" when it's a prior year), which
 * stays legible however old the timestamp is.
 *
 * The relative and absolute forms are produced by `Intl.RelativeTimeFormat` /
 * `Intl.DateTimeFormat` so they follow the passed `locale` (Android WebView,
 * Chromium SDK 28+, ships both). Callers pass the active i18n locale so the
 * label tracks the user's display language.
 *
 * @param now - Current time in milliseconds
 * @param timestamp - The timestamp to format
 * @param locale - BCP-47 tag (e.g. `"en"`, `"zh-CN"`); defaults to English
 */
export function formatRelativeTime(
  now: number,
  timestamp: number,
  locale = "en",
): string {
  const seconds = Math.floor((now - timestamp) / 1000);
  if (seconds < 60) return JUST_NOW[locale] ?? JUST_NOW.en;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return relativeFormatter(locale).format(-minutes, "minute");
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return relativeFormatter(locale).format(-hours, "hour");
  const days = Math.floor(hours / 24);
  if (days < 7) return relativeFormatter(locale).format(-days, "day");

  const date = new Date(timestamp);
  const withYear = date.getFullYear() !== new Date(now).getFullYear();
  return absoluteFormatter(locale, withYear).format(date);
}
