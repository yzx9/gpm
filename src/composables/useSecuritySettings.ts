// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { getAppConfig, type AppConfig, type LockMode } from "@/api";
import { inject, ref, type InjectionKey, type Ref } from "vue";

/**
 * Single cache for the security-related config the UI needs reactively. Today
 * that is the password-view auto-clear seconds (the only setting the frontend
 * *enforces*) plus the auto-lock `lockMode` — cached reactively so the activity
 * bumper (`useLockActivity`) can filter bumps without a per-event config fetch.
 * The backend still owns the auto-lock timer and the authoritative mode;
 * clipboard-clear is enforced backend-side too. `SettingsPage` and
 * `useSecretReveal` read from here so a settings change is visible everywhere
 * without each caller re-fetching `get_app_config`.
 *
 * Provided app-wide via `SECURITY_SETTINGS_KEY` (see `main.ts`); one instance,
 * loaded once from the backend and refreshed whenever a setting is applied
 * (`applySecurityConfig`). Tests construct their own via `createSecuritySettings()`.
 */

/** The reactive security-settings cache consumed by the UI. */
export interface SecuritySettingsState {
  /** Password-view auto-clear seconds (`0` ⇒ Never). */
  readonly viewClearSecs: Readonly<Ref<number>>;
  /** Cached auto-lock mode (Immediate default) — read by the activity bumper. */
  readonly lockMode: Readonly<Ref<LockMode>>;
  /** Load the cache from the backend once. Idempotent. */
  loadSecuritySettings: () => Promise<void>;
  /** Apply a freshly-fetched (or just-set) app config to the cache. */
  applySecurityConfig: (cfg: AppConfig) => void;
}

/** Default password-view auto-clear seconds (used when the backend omits it). */
const DEFAULT_VIEW_CLEAR_SECS = 45;

/** Injection key for the app-wide security-settings cache. */
export const SECURITY_SETTINGS_KEY: InjectionKey<SecuritySettingsState> =
  Symbol("SecuritySettings");

/**
 * Create a fresh security-settings cache. Production calls this once in
 * `main.ts` and provides it; tests call it per-case for isolation.
 */
export function createSecuritySettings(): SecuritySettingsState {
  const viewClearSecs = ref(DEFAULT_VIEW_CLEAR_SECS);
  // Cached auto-lock mode so the activity bumper can filter bumps reactively.
  const lockMode = ref<LockMode>("immediate");
  let initialized = false;

  /**
   * Load the security settings from the backend once. Idempotent. Call from
   * `App.vue` on mount so the view-clear timer is correct before any reveal.
   * A failure (e.g. pre-setup) leaves the defaults in place.
   */
  async function loadSecuritySettings() {
    if (initialized) return;
    initialized = true;
    try {
      applySecurityConfig(await getAppConfig());
    } catch {
      // Not configured yet, or the load failed — keep defaults.
    }
  }

  /** Apply a freshly-fetched (or just-set) app config to the cache. */
  function applySecurityConfig(cfg: AppConfig) {
    // null/undefined ⇒ default; 0 ⇒ 0 (Never — the caller skips its timer).
    viewClearSecs.value = cfg.view_clear_secs ?? DEFAULT_VIEW_CLEAR_SECS;
    // lock_mode absent ⇒ Immediate (matches LockMode's serde default).
    lockMode.value = cfg.lock_mode ?? "immediate";
  }

  return { viewClearSecs, lockMode, loadSecuritySettings, applySecurityConfig };
}

/**
 * Inject the app-wide security-settings cache. Must be called within a
 * component `setup()` under a tree that provided `SECURITY_SETTINGS_KEY`.
 * Throws if missing so a forgotten `provide` fails loudly.
 */
export function useSecuritySettings(): SecuritySettingsState {
  const s = inject(SECURITY_SETTINGS_KEY);
  if (!s) {
    throw new Error(
      "useSecuritySettings() requires SECURITY_SETTINGS_KEY to be provided",
    );
  }
  return s;
}
