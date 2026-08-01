// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

/** Shared setup helpers — URL classification + key truncation.
 *
 * Used by both the clone and create flows so they classify repo URLs and
 * truncate keys identically (previously duplicated across the setup components).
 */

/** Is `url` an SSH git remote (`ssh://…` or `user@host:path`), vs HTTPS? */
export function isSshUrl(url: string): boolean {
  const trimmed = url.trim();
  return (
    trimmed.startsWith("ssh://") ||
    (trimmed.includes("@") &&
      trimmed.includes(":") &&
      !trimmed.startsWith("http"))
  );
}

/** Truncate a long key for display, keeping a recognizable head + tail. */
export function truncateKey(key: string): string {
  if (key.length <= 24) return key;
  return `${key.slice(0, 12)}…${key.slice(-8)}`;
}
