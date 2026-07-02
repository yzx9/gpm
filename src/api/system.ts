// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import {
  addPluginListener,
  invoke,
  type PluginListener,
} from "@tauri-apps/api/core";

/**
 * Device/platform IPC — mirrors `src-tauri/src/app_config.rs` plus the local
 * `safe-area` and `screen-secure` Tauri plugins. These are the only frontend
 * calls that hit plugin commands (`plugin:<name>|<cmd>`) or `addPluginListener`;
 * centralizing them here keeps the plugin surface out of pages/composables.
 */

/** Safe-area window insets (status bar / nav bar / cutout), in CSS pixels. */
export interface SafeAreaInsets {
  top: number;
  bottom: number;
  left: number;
  right: number;
}

/** Persisted app-level config (`app.json`) — currently just the secure-screen flag. */
export interface AppConfig {
  secure_screen: boolean;
}

/**
 * Read the persisted app config. {@link AppConfig.secure_screen} is the master
 * screen-capture-protection toggle.
 */
export async function getAppConfig(): Promise<AppConfig> {
  return invoke<AppConfig>("get_app_config");
}

/**
 * Persist the master screen-capture-protection toggle (`set_secure_screen`).
 * Independent of the per-route plugin flag pushed by {@link setSecure}.
 */
export async function setSecureScreen(enabled: boolean): Promise<void> {
  await invoke("set_secure_screen", { enabled });
}

/**
 * Whether the `screen-secure` plugin is loaded (Android `FLAG_SECURE` support).
 * A compile-time-style platform fact reported by the backend — `true` on Android,
 * rejects/`false` on desktop. NOT inferred from invoke success.
 */
export async function screenSecureAvailable(): Promise<boolean> {
  return invoke<boolean>("screen_secure_available");
}

/**
 * Push the current `FLAG_SECURE` level for the route (`screen-secure` plugin).
 * Desktop / absent plugin: the invoke rejects and callers treat it as a no-op.
 */
export async function setSecure(secure: boolean): Promise<void> {
  await invoke("plugin:screen-secure|set_secure", { secure });
}

/** Read the live window insets once (`safe-area` plugin). Rejects on desktop. */
export async function getSafeAreaInsets(): Promise<SafeAreaInsets> {
  return invoke<SafeAreaInsets>("plugin:safe-area|get_insets");
}

/**
 * Subscribe to inset changes from the `safe-area` plugin. Best-effort on
 * edge-to-edge WebViews (the listener is unreliable there), so callers should
 * also re-pull via {@link getSafeAreaInsets} on layout events. Returns an
 * `unlisten` handle.
 */
export async function subscribeSafeArea(
  cb: (insets: SafeAreaInsets) => void,
): Promise<PluginListener> {
  return addPluginListener<SafeAreaInsets>(
    "safe-area",
    "safe-area-changed",
    cb,
  );
}
