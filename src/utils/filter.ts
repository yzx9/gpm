// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import type { Entry } from "../types";

/**
 * Filter entries by a search query, matching case-insensitively on name and path.
 */
export function filterEntries(entries: Entry[], query: string): Entry[] {
  const q = query.toLowerCase();
  if (!q) return entries;
  return entries.filter(
    (e) => e.name.toLowerCase().includes(q) || e.path.toLowerCase().includes(q),
  );
}
