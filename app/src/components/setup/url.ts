// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

/** Shared setup helpers — URL classification + key truncation.
 *
 * Used by both the clone and create flows so they classify repo URLs and
 * truncate keys identically (previously duplicated across the setup components).
 */

/** Is `url` an SSH git remote (`ssh://…` or `user@host:path`), vs HTTPS?
 * Scheme-qualified URLs of any other kind (`https://`, `git://`, `rsync://`…,
 * case-insensitive) never qualify. Mirrors the Rust `is_ssh_url` guard. */
export function isSshUrl(url: string): boolean {
  const trimmed = url.trim();
  const lower = trimmed.toLowerCase();
  if (lower.includes("://")) {
    return lower.startsWith("ssh://");
  }
  return trimmed.includes("@") && trimmed.includes(":");
}

/** Truncate a long key for display, keeping a recognizable head + tail. */
export function truncateKey(key: string): string {
  if (key.length <= 24) return key;
  return `${key.slice(0, 12)}…${key.slice(-8)}`;
}
