// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { onBackButtonPress } from "@tauri-apps/api/app";
import {
  addPluginListener,
  invoke,
  type PluginListener,
} from "@tauri-apps/api/core";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";

import type { LockMode, SecureScreenMode } from "./common";

// R085: generated from the Rust enums by `just gen-codegen`.
import type { BackgroundSyncCadence, GateIdle } from "./generated/app";

export type { BackgroundSyncCadence, GateIdle };

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
 *  - `secure_screen_mode`: three-state screen-capture-protection mode.
 *  - `locale`: display-language override. Absent (not `null`) ⇒ track system;
 *    `"en"` / `"zh-CN"` ⇒ pinned.
 *  - `theme_mode`: color-scheme override. Absent (not `null`) ⇒ track system
 *    (the CSS `prefers-color-scheme` media query); `"light"` / `"dark"` ⇒ pinned.
 *  - `lock_mode` / `view_clear_secs` / `clipboard_clear_secs` / `autosync` /
 *    `biometric_app_lock` / `gate_idle`: behavior prefs (absent ⇒ default). */
/** The canonical gate-idle default (mirrors the backend: absent ⇒ After 300s). */
export const DEFAULT_GATE_IDLE: GateIdle = { after: 300 };

/** The auto-lock idle duration restored when (re)entering "After idle" with no
 *  prior idle choice (the install default is Immediate, not an idle value). */
export const DEFAULT_LOCK_IDLE: LockMode = { idle: 60 };

/** The background-sync cadence used when periodic background sync is first
 *  enabled (the install default is "off", no cadence chosen). */
export const DEFAULT_BACKGROUND_SYNC_CADENCE: BackgroundSyncCadence = "6h";

export interface AppConfig {
  /** Persisted-schema version (one-shot migration gate). Absent ⇒ 1. */
  schema_version?: number;
  /** Three-state screen-capture protection (mirrors Rust `SecureScreenMode`).
   *  Absent ⇒ `"sensitive"`. */
  secure_screen_mode?: SecureScreenMode;
  locale?: string;
  /** Color-scheme override. Absent ⇒ track system; `"light"` / `"dark"` ⇒
   *  pinned (applied via a `<html data-theme>` attribute by `@/theme`). */
  theme_mode?: string;
  /** App auto-lock mode. Absent ⇒ Immediate. Mirrors Rust `LockMode`. */
  lock_mode?: LockMode;
  /** Password-view auto-clear seconds. Absent/null ⇒ default (45); 0 ⇒ never. */
  view_clear_secs?: number | null;
  /** Clipboard auto-clear seconds. Absent/null ⇒ default (45); 0 ⇒ never. */
  clipboard_clear_secs?: number | null;
  /** Per-device autosync: on (absent ⇒ true) ⇒ every save pull-write-pushes;
   *  off ⇒ saves stay local until a manual Sync publishes. */
  autosync?: boolean;
  /** Passive update-availability probe on cold start (RFC R090). Absent ⇒ true
   *  (on); off ⇒ the probe is skipped and the update dots never light. */
  update_check_enabled?: boolean;
  /** Latest release tag seen by the cold-start probe (e.g. `"v0.19.0"`), or
   *  absent until the first successful probe. Internal — the frontend reads it
   *  via {@link getUpdateStatus}, not this field; it rides along in `app.json`
   *  because the probe writes the same sealed file as the toggle. */
  latest_release?: string;
  /** When the cold-start probe last ran (Unix seconds). Absent until the first
   *  probe. Internal — drives the ≤1/day throttle. */
  release_probe_at?: number;
  /** The release tag the user acknowledged by opening About (absent until then).
   *  Scopes the Settings-entry dot; the About-page dot ignores it. Internal. */
  seen_release?: string;
  /** Periodic background-sync cadence. Absent ⇒ `"off"`. */
  background_sync?: BackgroundSyncCadence;
  /** Persisted intent for the app-launch biometric gate. **Write-only** — the
   *  Settings toggle + runtime gate read `getAppLockState` (Keystore truth),
   *  not this flag; it exists only as a persisted record. */
  biometric_app_lock?: boolean;
  /** App-launch-gate in-app idle timeout. Absent ⇒ the default (After 300s). */
  gate_idle?: GateIdle;
  /** Verbose-logging deadline as Unix seconds. Set + unexpired ⇒ the app logs at
   *  Debug this session (and on any relaunch within the window); absent/expired ⇒
   *  Info. Apply via {@link setVerbose}; check liveness via
   *  {@link isVerboseActive}. See RFC 0055. */
  verbose_until?: number;
}

/**
 * Read the persisted app config. {@link AppConfig.secure_screen_mode} is the
 * three-state screen-capture-protection mode.
 */
export async function getAppConfig(): Promise<AppConfig> {
  return invoke<AppConfig>("get_app_config");
}

/**
 * Persist the three-state screen-capture-protection mode
 * (`set_secure_screen_mode`). Returns the updated config; the caller re-applies
 * the current route's secure state on receipt. Independent of the per-route
 * plugin flag pushed by {@link setSecure}.
 */
export async function setSecureScreenMode(
  mode: SecureScreenMode,
): Promise<AppConfig> {
  return invoke<AppConfig>("set_secure_screen_mode", { mode });
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
 * Persist the color-scheme preference (`set_theme_mode`). `null` clears the
 * override (track system); `"light"` / `"dark"` pin it. Returns the updated
 * config; the caller re-applies the theme on receipt.
 */
export async function setThemeMode(mode: string | null): Promise<AppConfig> {
  return invoke<AppConfig>("set_theme_mode", { mode });
}

/**
 * Localized text for the verbose-revert OS notification. Passed to
 * {@link setVerbose} on enable; the backend stages it and posts it (from Rust,
 * not the WebView) when the window elapses, so the notice fires even if the app
 * is backgrounded. Mirrors how clipboard-notify takes its text from the frontend.
 */
export interface VerboseNotifyText {
  title: string;
  body: string;
}

/**
 * Turn verbose (Debug) logging on for a bounded window (~10 min), or off
 * (`set_verbose`). Returns the updated config; the backend re-applies the
 * runtime log gate so the level takes effect immediately. On enable, `revertNotify`
 * stages the localized text for the OS notification the deadline timer posts when
 * the window elapses. Verbose persists; the deadline auto-reverts to Info
 * (mid-session via the timer, or at the next launch if the process was killed).
 * See RFC 0055.
 */
export async function setVerbose(
  enabled: boolean,
  revertNotify?: VerboseNotifyText,
): Promise<AppConfig> {
  return invoke<AppConfig>("set_verbose", { enabled, revertNotify });
}

/**
 * Whether a verbose deadline is still in the future (`verbose_until` is Unix
 * seconds). Pure so both the Logs toggle and the boot notification share one
 * liveness check.
 */
export function isVerboseActive(verboseUntil: number | undefined): boolean {
  return typeof verboseUntil === "number" && verboseUntil * 1000 > Date.now();
}

/** Seconds remaining in the verbose window (`≤ 0` if expired/unset). Pure. */
export function verboseRemainingSecs(verboseUntil: number | undefined): number {
  if (typeof verboseUntil !== "number") return 0;
  return Math.max(0, verboseUntil - Math.floor(Date.now() / 1000));
}

/**
 * Post an OS notification (`tauri-plugin-notification`), requesting permission
 * first if not yet granted. Best-effort: any failure is swallowed —
 * notifications are non-critical. `POST_NOTIFICATIONS` is already declared and
 * typically granted via the clipboard-notify flow, so this normally does not
 * prompt. No-op-ish on desktop (the plugin posts a native notification there).
 */
export async function notifyOs(title: string, body?: string): Promise<void> {
  try {
    if (!(await isPermissionGranted())) {
      if ((await requestPermission()) !== "granted") return;
    }
    sendNotification({ title, body });
  } catch {
    // best-effort — a missing notification never fails the caller
  }
}

/**
 * Set the app auto-lock mode (`immediate` / `{ idle: secs }` / `never`). Returns
 * the updated config.
 */
export async function setLockMode(mode: LockMode): Promise<AppConfig> {
  return invoke<AppConfig>("set_lock_mode", { mode });
}

/**
 * Set the app-launch-gate in-app idle timeout (`"off"` = re-lock only on
 * background→foreground; `{ after: secs }` = re-lock after `secs` of foreground
 * idle). Returns the updated config.
 */
export async function setGateIdle(mode: GateIdle): Promise<AppConfig> {
  return invoke<AppConfig>("set_gate_idle", { mode });
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

/** Set the periodic background-sync cadence (`"off"` opts out). Returns the
 *  updated config. */
export async function setBackgroundSync(
  cadence: BackgroundSyncCadence,
): Promise<AppConfig> {
  return invoke<AppConfig>("set_background_sync", { cadence });
}

/** Cached update-probe result (RFC R090). `available` lights the About-page dot
 *  + the Update link; `unacknowledged` additionally lights the Settings-entry
 *  dot (the About-page dot ignores the ack). Read from the plaintext cache — no
 *  network. Mirrors the backend `UpdateStatus`. */
export interface UpdateStatus {
  /** A newer stable release exists than the built-in version. */
  available: boolean;
  /** A newer release exists that the user has not yet acknowledged by opening
   *  About (lights the Settings-entry dot only). */
  unacknowledged: boolean;
  /** The latest release tag seen (e.g. `"v0.19.0"`), or `null` if never probed. */
  latest_version: string | null;
}

/** Read the cached update-probe status (RFC R090). No network — the cold-start
 *  probe writes the cache (≤1/day); this reads it. Returns a quiet
 *  `{ available: false, ... }` when the check is off or unavailable
 *  (fail-closed). */
export async function getUpdateStatus(): Promise<UpdateStatus> {
  return invoke<UpdateStatus>("get_update_status");
}

/** Acknowledge the current latest release — records that the user opened About
 *  for this version, so the Settings-entry dot falls quiet. The About-page dot
 *  ignores the ack (RFC R090). Fire-and-forget from the About page on mount. */
export async function acknowledgeUpdate(): Promise<void> {
  await invoke("acknowledge_update");
}

/** Toggle the passive update check on/off (sealed in `app.json`, like autosync).
 *  Returns the updated config. */
export async function setUpdateCheck(enabled: boolean): Promise<AppConfig> {
  return invoke<AppConfig>("set_update_check", { enabled });
}

/** Take-once: whether a background sync left a divergence / authenticity-block
 *  attention marker (removing it). The foreground calls this on cold-start to
 *  decide whether to trigger a sync + surface the badge. */
export async function consumeSyncAttention(): Promise<boolean> {
  return invoke<boolean>("consume_sync_attention");
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
 * The platform gpm runs on — mirrors the backend `RuntimePlatform` enum (serde
 * `kebab-case` over the wire). `"unknown"` covers an unrecognized build target
 * and the pre-init state; features opt in per platform, so `"unknown"` activates
 * nothing. The bare-passthrough return relies on the backend's Rust
 * serialization test to pin these exact wire strings.
 */
export type RuntimePlatform =
  | "android"
  | "linux"
  | "macos"
  | "windows"
  | "unknown";

/**
 * General platform fact for UI gating (distinct from {@link screenSecureAvailable},
 * a screen-secure capability probe). Resolves to a concrete value once the
 * frontend's `usePlatform` init runs; `"unknown"` until then and on any
 * unrecognized build.
 */
export async function runtimePlatform(): Promise<RuntimePlatform> {
  return invoke<RuntimePlatform>("runtime_platform");
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
