// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { getAppConfig } from "@/api";

/**
 * Color-scheme (light/dark) application.
 *
 * The default "System" mode is handled entirely by CSS — `src/style.css` has a
 * `@media (prefers-color-scheme: dark)` block that overrides the `:root` color
 * variables, so it is zero-JS, zero-flash, and reacts live when the OS theme
 * changes. A **pinned** Light/Dark override layers a `data-theme` attribute on
 * `<html>`: the same dark variables are forced via `:root[data-theme="dark"]`,
 * and a pinned Light opts out of the dark media query (`:root:not([data-theme=
 * "light"])`).
 *
 * JS therefore only ever sets or clears that one attribute. The pinned
 * preference no longer flashes at cold start: the Rust setup closure reads
 * `pref.json` and bakes `data-theme` into a per-window init script that runs
 * before first paint (see `theme_init_script` in `app_config.rs`). `reconcile`
 * is the post-mount authoritative correction — it confirms the init script's
 * value and fixes the rare case where `pref.json` was unreadable at setup.
 */

/** A theme the settings picker offers: track the OS, or pin light/dark. */
export type ThemeMode = "system" | "light" | "dark";

/**
 * Map a persisted `theme_mode` value to a picker mode. Absent / unknown
 * (including a hand-edited `"system"` or garbage) degrades to "system" rather
 * than poisoning the UI — mirroring how `normalizeSupported` defends the locale.
 */
export function normalizeThemeMode(raw: string | null | undefined): ThemeMode {
  if (raw === "light" || raw === "dark") return raw;
  return "system";
}

/**
 * Apply a theme mode to `<html data-theme>`: clear the attribute for "system"
 * (so the CSS media query governs), or set it to pin light/dark. Idempotent.
 *
 * Whitelists the pinned values rather than trusting the `ThemeMode` type: a
 * non-matching value (a future caller bypassing the type) must never land on
 * the attribute, or neither `[data-theme="dark"]` nor `[data-theme="light"]`
 * matches and `:not([data-theme="light"])` excludes it — silently sticking the
 * app on the light tokens regardless of intent. Anything other than
 * light/dark clears the attribute (the safe System fallback).
 */
export function applyTheme(mode: ThemeMode): void {
  const root = document.documentElement;
  if (mode === "light" || mode === "dark") {
    root.dataset.theme = mode;
  } else {
    delete root.dataset.theme;
  }
}

/**
 * Read the persisted `theme_mode` from the backend and apply it. The Rust
 * setup closure already baked the pinned theme into a pre-paint init script,
 * so this is an authoritative safety net — it confirms that value and corrects
 * the rare case where `pref.json` was unreadable at setup (corrupt file, first
 * launch). Failures (no backend in pure-Vite dev, IPC blip) are swallowed so
 * the app keeps the CSS-driven System default rather than blanking.
 */
export async function reconcileThemeFromBackend(): Promise<void> {
  try {
    const cfg = await getAppConfig();
    applyTheme(normalizeThemeMode(cfg.theme_mode));
  } catch (e) {
    // Keep the CSS default (System).
    console.debug("[theme] reconcile failed", e);
  }
}
