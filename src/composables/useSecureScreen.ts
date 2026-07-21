// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import {
  getAppConfig,
  setSecureScreenMode as persistSecureScreenMode,
  screenSecureAvailable,
  setSecure,
} from "@/api";
import type { SecureScreenMode } from "@/api/common";
import { inject, ref, type InjectionKey, type Ref } from "vue";

/**
 * Per-page screen-capture protection (Android `FLAG_SECURE`) state.
 *
 * `secureScreenMode` is the three-state master setting — `"off"` / `"sensitive"`
 * (default) / `"always"` — persisted in the backend `app.json`.
 * `secureAvailable` is a compile-time platform fact from
 * `screen_secure_available()` — NOT inferred from invoke success — so a broken
 * Android plugin is never mistaken for desktop (which would fail open).
 *
 * Effective `FLAG_SECURE` per mode:
 *  - `off` — never secure (the user explicitly allowed capture, including the
 *    unlock overlay);
 *  - `always` — every screen secure at all times;
 *  - `sensitive` — a route is secure when it bears the secret flag, OR the
 *    global unlock overlay is up.
 *
 * The overlay collects the identity passphrase — a credential that must never be
 * capturable — so under `"sensitive"` it forces `FLAG_SECURE` on even on an
 * otherwise-capturable route like `/entries`.
 *
 * `App.vue` calls `initSecureScreen` on mount to load availability + the mode
 * and reconcile the current route. The boot default in `MainActivity.onCreate`
 * keeps every screen secure until that runs.
 *
 * Provided app-wide via `SECURE_SCREEN_KEY` (see `main.ts`); the router guards
 * hold the instance directly (they run outside setup). Tests construct their
 * own via `createSecureScreen()`.
 */

/** Reactive screen-capture-protection state + the route/overlay drivers. */
export interface SecureScreenState {
  /** Three-state master mode (default `"sensitive"`, persisted via
   *  `setSecureScreenMode`). Mutable so tests can drive it without invoking the
   *  persisting setter. */
  secureScreenMode: Ref<SecureScreenMode>;
  /** Platform fact from `screen_secure_available()` (NOT inferred from invoke
   *  success). Mutable for the same test reason as `secureScreenMode`. */
  secureAvailable: Ref<boolean>;
  /** Load availability + the master mode once, then reconcile. Idempotent. */
  initSecureScreen: () => Promise<void>;
  /** Pre-paint raise for a navigation transition (covers the departing page). */
  raiseSecureForRoute: (needsCover: boolean) => Promise<boolean>;
  /** Settle the flag to the arriving route's level (after paint). */
  applySecureForRoute: (routeSecure: boolean) => Promise<boolean>;
  /** Reflect whether the global unlock overlay is up; re-applies immediately. */
  setSecureOverlay: (active: boolean) => Promise<boolean>;
  /** Persist the master mode, then re-apply. Reverts on failure. */
  setSecureScreenMode: (mode: SecureScreenMode) => Promise<boolean>;
}

/** Seed options for `createSecureScreen` (test/seed only; production passes none). */
export interface CreateSecureScreenOptions {
  /** Start with the plugin reported available (Android). Default false (desktop). */
  available?: boolean;
  /** Start with a specific mode (default `"sensitive"`). */
  mode?: SecureScreenMode;
}

/** Injection key for the app-wide screen-capture-protection state. */
export const SECURE_SCREEN_KEY: InjectionKey<SecureScreenState> =
  Symbol("SecureScreen");

/**
 * Create a fresh screen-capture-protection instance. Production calls this once
 * in `main.ts` and provides it; tests call it per-case for isolation.
 */
export function createSecureScreen(
  opts: CreateSecureScreenOptions = {},
): SecureScreenState {
  const secureScreenMode = ref<SecureScreenMode>(opts.mode ?? "sensitive");
  const secureAvailable = ref(opts.available === true);
  let currentRouteSecure = false;
  let overlayActive = false;
  let initialized = false;

  /**
   * Effective `FLAG_SECURE` level for a given route-level secret flag. `off`
   * never secures; `always` always secures; `sensitive` secures when the route
   * bears the secret flag or the unlock overlay is up. Exhaustive over the
   * `SecureScreenMode` union so a future mode forces an update here.
   */
  function desiredSecure(routeLevel: boolean): boolean {
    switch (secureScreenMode.value) {
      case "off":
        return false;
      case "always":
        return true;
      case "sensitive":
        return routeLevel || overlayActive;
    }
  }

  /** Push `FLAG_SECURE` for a route level. Desktop / absent plugin: no-op (`true`). */
  async function pushFlag(routeLevel: boolean): Promise<boolean> {
    if (!secureAvailable.value) return true; // desktop / plugin absent: no-op
    try {
      await setSecure(desiredSecure(routeLevel));
      return true;
    } catch {
      return false;
    }
  }

  /** Re-apply `FLAG_SECURE` for the last settled route (plus the overlay state). */
  async function applyCurrentRoute(): Promise<boolean> {
    return pushFlag(currentRouteSecure);
  }

  /**
   * Load availability + the master mode once, then reconcile the current route.
   * Idempotent. Call from `App.vue` on mount. An absent or unrecognized backend
   * value (e.g. `"unknown"`, a forward-compat sink from a newer build) resolves
   * to `"sensitive"`.
   */
  async function initSecureScreen() {
    if (initialized) return;
    initialized = true;
    try {
      secureAvailable.value = await screenSecureAvailable();
    } catch {
      // Couldn't confirm availability. On Android this command always returns
      // `true`, so a rejection means the bridge is flaky — NOT "desktop". Assume
      // available so subsequent calls are ATTEMPTED and fail-closed (the guard
      // aborts secret routes) rather than silently no-op'd (fail-open).
      secureAvailable.value = true;
    }
    try {
      const cfg = await getAppConfig();
      const raw = cfg.secure_screen_mode;
      secureScreenMode.value =
        raw === "off" || raw === "always" ? raw : "sensitive";
    } catch {
      // Backend unavailable (e.g. pre-setup) — keep the default "sensitive".
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
  async function raiseSecureForRoute(needsCover: boolean): Promise<boolean> {
    return pushFlag(needsCover);
  }

  /**
   * Settle the flag to the arriving route's level, after its component has
   * mounted/painted (call from `router.afterEach` + `nextTick`). Also used
   * directly outside transitions (`initSecureScreen`, `setSecureScreenMode`).
   * Returns whether the plugin call succeeded.
   */
  async function applySecureForRoute(routeSecure: boolean): Promise<boolean> {
    currentRouteSecure = routeSecure;
    return applyCurrentRoute();
  }

  /**
   * Reflect whether the global unlock overlay is up. Under `"sensitive"` the
   * overlay collects the identity passphrase, so raising it forces `FLAG_SECURE`
   * on (see `desiredSecure`) even on a capturable route. Re-applies immediately;
   * returns the plugin result (the `App.vue` watcher ignores it).
   */
  function setSecureOverlay(active: boolean): Promise<boolean> {
    overlayActive = active;
    return applyCurrentRoute();
  }

  /**
   * Persist the master mode, then re-apply the current route's secure state.
   * Returns `false` (reverting the in-memory ref and re-applying the route) if
   * persistence failed, so the UI never shows a mode that didn't actually save —
   * UI/disk/window stay in sync instead of desyncing on a failed write.
   */
  async function setSecureScreenMode(mode: SecureScreenMode): Promise<boolean> {
    const prev = secureScreenMode.value;
    secureScreenMode.value = mode;
    try {
      await persistSecureScreenMode(mode);
    } catch {
      // Persistence failed — revert to the last-known-persisted value and
      // re-push FLAG_SECURE for it, so the window never keeps the optimistic
      // value (a navigation mid-IPC could otherwise leave a secret capturable).
      secureScreenMode.value = prev;
      await applyCurrentRoute();
      return false;
    }
    await applyCurrentRoute();
    return true;
  }

  return {
    secureScreenMode,
    secureAvailable,
    initSecureScreen,
    applySecureForRoute,
    raiseSecureForRoute,
    setSecureOverlay,
    setSecureScreenMode,
  };
}

/**
 * Inject the app-wide screen-capture-protection state. Must be called within a
 * component `setup()` under a tree that provided `SECURE_SCREEN_KEY`. Throws if
 * missing so a forgotten `provide` fails loudly. (Router guards in `main.ts`
 * use the held instance directly — they run outside setup.)
 */
export function useSecureScreen(): SecureScreenState {
  const s = inject(SECURE_SCREEN_KEY);
  if (!s) {
    throw new Error(
      "useSecureScreen() requires SECURE_SCREEN_KEY to be provided",
    );
  }
  return s;
}
