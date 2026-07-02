// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import {
  getSafeAreaInsets,
  subscribeSafeArea,
  type SafeAreaInsets,
} from "@/api";

function applyInsets(insets: SafeAreaInsets): void {
  document.documentElement.style.setProperty(
    "--safe-area-inset-top",
    `${insets.top}px`,
  );
  document.documentElement.style.setProperty(
    "--safe-area-inset-bottom",
    `${insets.bottom}px`,
  );
  document.documentElement.style.setProperty(
    "--safe-area-inset-left",
    `${insets.left}px`,
  );
  document.documentElement.style.setProperty(
    "--safe-area-inset-right",
    `${insets.right}px`,
  );
}

/**
 * Apply safe-area insets and keep them current.
 *
 * On Android, the `safe-area` Tauri plugin exposes `get_insets`, which reads the
 * live window insets directly. We pull it once at startup and again on rotation
 * (orientationchange/resize).
 *
 * We also subscribe to the plugin's `safe-area-changed` event as a best-effort
 * signal, but do NOT rely on it: the plugin's `OnApplyWindowInsetsListener` is
 * unreliable in this edge-to-edge WebView (it doesn't consistently fire on
 * rotation), so the re-query on layout events is what actually keeps the insets
 * correct.
 *
 * On desktop, the plugin is absent; the `invoke` rejects and
 * CSS `var()` fallbacks of `0px` apply.
 */
export async function applySafeAreaInsets(): Promise<void> {
  try {
    const insets = await getSafeAreaInsets();
    applyInsets(insets);

    await subscribeSafeArea(applyInsets);

    // Re-pull live insets on rotation. `get_insets` reads the current committed
    // insets, so this stays correct without depending on the plugin's listener.
    // Not bound to visualViewport.resize: the keyboard (IME) doesn't change the
    // status/nav-bar/cutout insets the plugin reports, so that would only churn
    // the IPC bridge with identical values.
    const refresh = (): void => {
      getSafeAreaInsets()
        .then(applyInsets)
        .catch(() => {
          /* plugin gone (e.g. teardown) — keep last insets */
        });
    };
    window.addEventListener("orientationchange", refresh);
    window.addEventListener("resize", refresh);
  } catch {
    // Desktop: plugin not registered, CSS fallback applies
  }
}
