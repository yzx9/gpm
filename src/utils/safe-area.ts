// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { invoke, addPluginListener } from "@tauri-apps/api/core";

interface SafeAreaInsets {
  top: number;
  bottom: number;
}

function applyInsets(insets: SafeAreaInsets): void {
  document.documentElement.style.setProperty(
    "--safe-area-inset-top",
    `${insets.top}px`,
  );
  document.documentElement.style.setProperty(
    "--safe-area-inset-bottom",
    `${insets.bottom}px`,
  );
}

/**
 * Apply safe-area insets and listen for dynamic changes.
 *
 * On Android, the `safe-area` Tauri plugin provides insets via:
 * 1. An initial `get_insets` call
 * 2. A `safe-area-changed` event on rotation, keyboard show/hide, etc.
 *
 * On desktop, the plugin is absent; the `invoke` rejects and
 * CSS `var()` fallbacks of `0px` apply.
 */
export async function applySafeAreaInsets(): Promise<void> {
  try {
    const insets = await invoke<SafeAreaInsets>("plugin:safe-area|get_insets");
    applyInsets(insets);

    await addPluginListener<SafeAreaInsets>(
      "safe-area",
      "safe-area-changed",
      applyInsets,
    );
  } catch {
    // Desktop: plugin not registered, CSS fallback applies
  }
}
