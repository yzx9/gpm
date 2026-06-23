// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { RepoConfig } from "../types";

/**
 * Single cache for the security-related config the UI needs reactively — today
 * just the password-view auto-clear seconds (the only one the frontend owns;
 * lock-mode and clipboard-clear are enforced backend-side). `SettingsPage` and
 * `useSecretReveal` both read from here so a settings change is visible
 * everywhere without each caller re-fetching `get_config`.
 *
 * Module-scoped: one app-wide cache, loaded once from the backend and refreshed
 * whenever a setting is applied (`applySecurityConfig`).
 */
const DEFAULT_VIEW_CLEAR_SECS = 45;

const viewClearSecs = ref(DEFAULT_VIEW_CLEAR_SECS);
let initialized = false;

export function useSecuritySettings() {
  return { viewClearSecs, loadSecuritySettings, applySecurityConfig };
}

/**
 * Load the security settings from the backend once. Idempotent. Call from
 * `App.vue` on mount so the view-clear timer is correct before any reveal.
 * A failure (e.g. pre-setup) leaves the defaults in place.
 */
async function loadSecuritySettings() {
  if (initialized) return;
  initialized = true;
  try {
    applySecurityConfig(await invoke<RepoConfig>("get_config"));
  } catch {
    // Not configured yet, or the load failed — keep defaults.
  }
}

/** Apply a freshly-fetched (or just-set) repo config to the cache. */
function applySecurityConfig(rc: RepoConfig) {
  // null/undefined ⇒ default; 0 ⇒ 0 (Never — the caller skips its timer).
  viewClearSecs.value = rc.view_clear_secs ?? DEFAULT_VIEW_CLEAR_SECS;
}

/** Test-only: reset the module singleton between cases. */
export function __resetSecuritySettingsForTests() {
  initialized = false;
  viewClearSecs.value = DEFAULT_VIEW_CLEAR_SECS;
}
