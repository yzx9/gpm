// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { onBackButtonPress } from "@tauri-apps/api/app";
import {
  addPluginListener,
  invoke,
  type PluginListener,
} from "@tauri-apps/api/core";

import type { LockMode } from "./common";

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

/** Persisted app-level config (`app.json`) — the app-scoped (non-repo)
 *  preferences. Plaintext on disk (not sealed): `locale` must be readable
 *  before unlock for the first-paint injection + app-lock biometric screen, so
 *  the whole file stays master-key-independent. The behavior prefs moved here
 *  from `RepoConfig` in the RFC 0038 scope split.
 *  - `secure_screen`: master screen-capture-protection toggle.
 *  - `locale`: display-language override. Absent (not `null`) ⇒ track system;
 *    `"en"` / `"zh-CN"` ⇒ pinned.
 *  - `lock_mode` / `view_clear_secs` / `clipboard_clear_secs` / `autosync` /
 *    `biometric_app_lock`: behavior prefs (absent ⇒ default). */
export interface AppConfig {
  /** Persisted-schema version (one-shot migration gate). Absent ⇒ 1. */
  schema_version?: number;
  secure_screen: boolean;
  locale?: string;
  /** App auto-lock mode. Absent ⇒ Immediate. Mirrors Rust `LockMode`. */
  lock_mode?: LockMode;
  /** Password-view auto-clear seconds. Absent/null ⇒ default (45); 0 ⇒ never. */
  view_clear_secs?: number | null;
  /** Clipboard auto-clear seconds. Absent/null ⇒ default (45); 0 ⇒ never. */
  clipboard_clear_secs?: number | null;
  /** Per-device autosync: on (absent ⇒ true) ⇒ every save pull-write-pushes;
   *  off ⇒ saves stay local until a manual Sync publishes. */
  autosync?: boolean;
  /** Persisted intent for the app-launch biometric gate. **Write-only** — the
   *  Settings toggle + runtime gate read `getAppLockState` (Keystore truth),
   *  not this flag; it exists only as a persisted record. */
  biometric_app_lock?: boolean;
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
 * Persist the display-language preference (`set_locale_pref`). `null` clears
 * the override (track system); `"en"` / `"zh-CN"` pin it. Returns the updated
 * config.
 */
export async function setLocalePref(locale: string | null): Promise<AppConfig> {
  return invoke<AppConfig>("set_locale_pref", { locale });
}

/**
 * Set the app auto-lock mode (`immediate` / `{ idle: secs }` / `never`). Returns
 * the updated config.
 */
export async function setLockMode(mode: LockMode): Promise<AppConfig> {
  return invoke<AppConfig>("set_lock_mode", { mode });
}

/**
 * Set the password-view auto-clear override (`null` = default, `0` = never).
 * Returns the updated config.
 */
export async function setViewClearSecs(
  secs: number | null,
): Promise<AppConfig> {
  return invoke<AppConfig>("set_view_clear_secs", { secs });
}

/**
 * Set the clipboard auto-clear override (`null` = default, `0` = never). Returns
 * the updated config.
 */
export async function setClipboardClearSecs(
  secs: number | null,
): Promise<AppConfig> {
  return invoke<AppConfig>("set_clipboard_clear_secs", { secs });
}

/**
 * Set per-save autosync (`true` ⇒ every save pull-write-pushes; `false` ⇒ saves
 * stay local until a manual Sync). Returns the updated config.
 */
export async function setAutosync(enabled: boolean): Promise<AppConfig> {
  return invoke<AppConfig>("set_autosync", { enabled });
}

/**
 * The authoritative locale the app should render in (explicit override if set
 * and supported, else the normalized system locale). The frontend reconciles
 * against the best-effort injected value at boot via this command.
 */
export async function resolvedLocale(): Promise<string> {
  return invoke<string>("resolved_locale");
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

/**
 * Subscribe to the Android back button (`back-button` event). Each press while
 * subscribed calls `cb` instead of navigating the webview (the default
 * `app.tauri.AppPlugin` behavior). Android-only in effect — on desktop this
 * registers an idle listener that never fires. Returns the plugin listener;
 * call `.unregister()` to release it back to Tauri's default back behavior.
 */
export async function subscribeBackButton(
  cb: () => void,
): Promise<PluginListener> {
  return onBackButtonPress(cb);
}
