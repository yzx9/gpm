// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import type { RouteLocationRaw, Router } from "vue-router";

/**
 * Pop one entry off the navigation stack, falling back to `fallback` when there
 * is nothing to pop.
 *
 * vue-router's browser history IS the nav stack. In-app Back buttons should pop
 * it (`router.back`), not append a fixed destination (`router.push`) — push
 * pollutes the history and makes Android system back loop through stale entries
 * (the back button ends up returning to previously-visited pages instead of the
 * logical previous one).
 *
 * `window.history.state.position` is vue-router's cursor (0 at the initial
 * entry, +1 per push, preserved on replace). At a deep-link root (position 0)
 * there is no previous entry to pop; use `router.replace` (not push) so the
 * fallback becomes the new root instead of stranding the deep-link page one
 * back-press away.
 */
export function navBack(router: Router, fallback: RouteLocationRaw): void {
  const pos =
    (window.history.state as { position?: number } | null)?.position ?? 0;
  if (pos > 0) router.back();
  else router.replace(fallback);
}
