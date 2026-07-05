// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

const SHORT_MONTHS = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
];

/**
 * Format a timestamp as a compact, human-readable time label.
 *
 * Recent timestamps use a relative form ("just now", "5m ago", "3h ago",
 * "2d ago"). Past a week the relative form stops carrying meaning — "249h
 * ago" requires arithmetic to parse — so we fall back to an absolute calendar
 * date ("Mar 15", or "Mar 15, 2024" when it's a prior year), which stays
 * legible however old the timestamp is.
 *
 * @param now - Current time in milliseconds
 * @param timestamp - The timestamp to format
 */
export function formatRelativeTime(now: number, timestamp: number): string {
  const seconds = Math.floor((now - timestamp) / 1000);
  if (seconds < 60) return "just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d ago`;

  const date = new Date(timestamp);
  const month = SHORT_MONTHS[date.getMonth()];
  const day = date.getDate();
  if (date.getFullYear() === new Date(now).getFullYear()) {
    return `${month} ${day}`;
  }
  return `${month} ${day}, ${date.getFullYear()}`;
}
