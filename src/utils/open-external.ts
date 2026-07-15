// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { openUrl } from "@tauri-apps/plugin-opener";

/**
 * Open an external (https) URL in the system browser — the supported path in
 * Tauri production (where a raw `<a target="_blank">` either no-ops or
 * navigates the WebView off the SPA). Falls back to a new browser tab when not
 * running under Tauri (pure Vite dev server, vitest), so the same call works
 * everywhere.
 *
 * Always pair with `@click.prevent` on the anchor so the WebView never
 * performs its own (uncontrolled) navigation alongside this.
 */
export async function openExternal(href: string): Promise<void> {
  try {
    await openUrl(href);
  } catch {
    window.open(href, "_blank", "noopener,noreferrer");
  }
}
