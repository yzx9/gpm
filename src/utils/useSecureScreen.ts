// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

/**
 * Per-page screen-capture protection (Android `FLAG_SECURE`) state.
 *
 * `secureScreen` is the master toggle (default ON, persists in the backend
 * `app.json`). `secureAvailable` is a compile-time platform fact from
 * `screen_secure_available()` — NOT inferred from invoke success — so a broken
 * Android plugin is never mistaken for desktop (which would fail open).
 *
 * Two surfaces can force `FLAG_SECURE` on regardless of the current route:
 *  - a navigation transition away from a secret page (covered during the swap —
 *    `raiseSecureForRoute` in `beforeEach`, settled in `afterEach`), and
 *  - the global unlock overlay (`overlayActive`), which collects the identity
 *    passphrase — a credential that must never be capturable, even on an
 *    otherwise-capturable route like `/entries`.
 *
 * Effective flag = `secureScreen && (currentRouteSecure || overlayActive)`.
 * `App.vue` calls `initSecureScreen` on mount to load availability + the toggle
 * and reconcile the current route. The boot default in `MainActivity.onCreate`
 * keeps every screen secure until that runs.
 */
const secureScreen = ref(true);
const secureAvailable = ref(false);
let currentRouteSecure = false;
let overlayActive = false;
let initialized = false;

export function useSecureScreen() {
  return {
    secureScreen,
    secureAvailable,
    initSecureScreen,
    applySecureForRoute,
    raiseSecureForRoute,
    setSecureOverlay,
    setSecureScreen,
  };
}

/**
 * Effective `FLAG_SECURE` level for a given route-level secret flag. The unlock
 * overlay is itself a secret surface (it collects the identity passphrase), so
 * it forces secure-on even on a non-secret route.
 */
function desiredSecure(routeLevel: boolean): boolean {
  return secureScreen.value && (routeLevel || overlayActive);
}

/** Push `FLAG_SECURE` for a route level. Desktop / absent plugin: no-op (`true`). */
async function pushFlag(routeLevel: boolean): Promise<boolean> {
  if (!secureAvailable.value) return true; // desktop / plugin absent: no-op
  try {
    await invoke("plugin:screen-secure|set_secure", {
      secure: desiredSecure(routeLevel),
    });
    return true;
  } catch {
    return false;
  }
}

/**
 * Load availability + the master toggle once, then reconcile the current route.
 * Idempotent. Call from `App.vue` on mount.
 */
async function initSecureScreen() {
  if (initialized) return;
  initialized = true;
  try {
    secureAvailable.value = await invoke<boolean>("screen_secure_available");
  } catch {
    // Couldn't confirm availability. On Android this command always returns
    // `true`, so a rejection means the bridge is flaky — NOT "desktop". Assume
    // available so subsequent calls are ATTEMPTED and fail-closed (the guard
    // aborts secret routes) rather than silently no-op'd (fail-open).
    secureAvailable.value = true;
  }
  try {
    const cfg = await invoke<{ secure_screen: boolean }>("get_app_config");
    secureScreen.value = cfg.secure_screen;
  } catch {
    // Backend unavailable (e.g. pre-setup) — keep the default ON.
  }
  await applyCurrentRoute();
}

/**
 * Pre-paint raise for a navigation: cover BOTH the departing and arriving page
 * so a secret page being navigated away from is never shown unprotected during
 * the swap. Does NOT commit `currentRouteSecure`; the guard settles that in
 * `afterEach` via `applySecureForRoute`. Returns whether the plugin call
 * succeeded; desktop (not available) returns `true` as a no-op. On Android,
 * `false` for a secret-bearing transition is a real failure the guard aborts on.
 */
export async function raiseSecureForRoute(
  needsCover: boolean,
): Promise<boolean> {
  return pushFlag(needsCover);
}

/**
 * Settle the flag to the arriving route's level, after its component has
 * mounted/painted (call from `router.afterEach` + `nextTick`). Also used
 * directly outside transitions (`initSecureScreen`, `setSecureScreen`). Returns
 * whether the plugin call succeeded.
 */
export async function applySecureForRoute(
  routeSecure: boolean,
): Promise<boolean> {
  currentRouteSecure = routeSecure;
  return applyCurrentRoute();
}

/** Re-apply `FLAG_SECURE` for the last settled route (plus the overlay state). */
async function applyCurrentRoute(): Promise<boolean> {
  return pushFlag(currentRouteSecure);
}

/**
 * Reflect whether the global unlock overlay is up. The overlay collects the
 * identity passphrase, so raising it forces `FLAG_SECURE` on (see
 * `desiredSecure`) even on a capturable route. Re-applies immediately; returns
 * the plugin result (the `App.vue` watcher ignores it).
 */
export function setSecureOverlay(active: boolean): Promise<boolean> {
  overlayActive = active;
  return applyCurrentRoute();
}

/**
 * Persist the master toggle, then re-apply the current route's secure state.
 * Returns `false` (and reverts the in-memory ref) if persistence failed, so the
 * UI never shows a toggle that didn't actually save — UI/disk/window stay in
 * sync instead of desyncing on a failed write.
 */
export async function setSecureScreen(enabled: boolean): Promise<boolean> {
  const prev = secureScreen.value;
  secureScreen.value = enabled;
  try {
    await invoke("set_secure_screen", { enabled });
  } catch {
    // Persistence failed — revert to the last-known-persisted value so the ref
    // tracks disk, not an orphaned optimistic write.
    secureScreen.value = prev;
    return false;
  }
  await applyCurrentRoute();
  return true;
}

/** Test-only: reset the module singleton between cases. */
export function __resetSecureScreenForTests() {
  initialized = false;
  secureScreen.value = true;
  secureAvailable.value = false;
  currentRouteSecure = false;
  overlayActive = false;
}
