// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

/**
 * Read safe-area insets from the Android-side JS interface and set CSS variables.
 *
 * On Android, `MainActivity` exposes `window.GpmInsets` with `getTop()` and
 * `getBottom()` returning the status bar / navigation bar heights in CSS pixels.
 * On desktop the interface is absent and this function is a no-op — the CSS
 * `var()` fallback of `0px` applies.
 */
export function applySafeAreaInsets() {
  const insets = (
    window as unknown as {
      GpmInsets?: { getTop: () => number; getBottom: () => number };
    }
  ).GpmInsets;
  if (!insets) return;

  document.documentElement.style.setProperty(
    "--safe-area-inset-top",
    `${insets.getTop()}px`,
  );
  document.documentElement.style.setProperty(
    "--safe-area-inset-bottom",
    `${insets.getBottom()}px`,
  );
}
