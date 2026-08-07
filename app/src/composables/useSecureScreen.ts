// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import {
  getAppConfig,
  setSecureScreenMode as persistSecureScreenMode,
  screenSecureAvailable,
  setSecure,
} from "@/api";
import type { SecureScreenMode } from "@/api/common";
import { inject, ref, type InjectionKey, type Ref } from "vue";

/**
 * Screen-capture protection (Android `FLAG_SECURE`) state — component-level
 * (R031).
 *
 * `secureScreenMode` is the three-state master setting — `"off"` / `"sensitive"`
 * (default) / `"always"` — persisted in the backend `app.json`.
 * `secureAvailable` is a compile-time platform fact from
 * `screen_secure_available()` — NOT inferred from invoke success — so a broken
 * Android plugin is never mistaken for desktop (which would fail open).
 *
 * Effective `FLAG_SECURE` per mode:
 *  - `off` — component claims are ignored (the user explicitly allowed capture,
 *    including of a revealed secret); only the credential overlay forces the
 *    flag (see below);
 *  - `always` — every screen secure at all times;
 *  - `sensitive` — the window is secure while ANY component holds a claim
 *    (a secret is on screen), OR the global unlock overlay is up.
 *
 * Claims are component-scoped (see `useSecureClaim`); this singleton only ORs
 * the live claim count into the single window-level bit. The credential
 * overlays (UnlockModal/AppLockOverlay) collect a passphrase / gate the master
 * key, so under every mode `overlayActive` forces `FLAG_SECURE` on — even under
 * `"off"` and on an otherwise-capturable route like `/entries`.
 *
 * `App.vue` calls `initSecureScreen` on mount to load availability + the mode
 * and reconcile. The boot default in `MainActivity.onCreate` keeps every screen
 * secure until that runs.
 *
 * Provided app-wide via `SECURE_SCREEN_KEY` (see `main.ts`); `useSecureClaim`
 * holds the instance via `useSecureScreen()` (it runs in component setup).
 * Tests construct their own via `createSecureScreen()`.
 */

/** Reactive screen-capture-protection state + the claim/overlay drivers. */
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
  /** Reset the one-shot latch and re-init from the backend. Use after an
   * app-unlock: the cold-start `initSecureScreen` read the default "sensitive"
   * (the sealed `secure_screen_mode` isn't readable pre-unlock); this re-reads
   * the real mode and re-applies FLAG_SECURE. */
  reload: () => Promise<void>;
  /** Reflect whether the global unlock overlay is up; re-applies immediately. */
  setSecureOverlay: (active: boolean) => Promise<boolean>;
  /** Persist the master mode, then re-apply. Reverts on failure. */
  setSecureScreenMode: (mode: SecureScreenMode) => Promise<boolean>;
  /** Acquire one component-level claim (raises FLAG_SECURE). Returns false if the
   *  plugin call failed — the caller must not render the secret. */
  acquireClaim: () => Promise<boolean>;
  /** Release one component-level claim (idempotent, floored at 0). */
  releaseClaim: () => Promise<boolean>;
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
  let overlayActive = false;
  let initialized = false;
  // Component-level claims: each secret-bearing component acquires before it
  // renders a secret and releases once the secret is gone (see `useSecureClaim`).
  // FLAG_SECURE is a single window bit, so this is an OR over all live claims —
  // a count, not a boolean. Under `"sensitive"` any claim > 0 secures the window.
  let claimCount = 0;

  /**
   * Effective `FLAG_SECURE` level. Exhaustive over `SecureScreenMode` so a
   * future mode forces an update here.
   *  - `off` — only the credential overlay secures (claims ignored; the user
   *    opted into capture);
   *  - `always` — always secure;
   *  - `sensitive` — secure while any claim is held OR the overlay is up.
   */
  function desiredSecure(): boolean {
    switch (secureScreenMode.value) {
      case "off":
        // The credential overlays (app-lock + identity unlock) must NEVER be
        // capturable, even when the user allowed capture elsewhere — so "off"
        // still secures while an overlay is up. Driven by runtime overlay state,
        // so a tampered stored "off" can't disable it, and the warm-relock
        // AppLockOverlay (mounted on resume) is secured regardless of mode.
        return overlayActive;
      case "always":
        return true;
      case "sensitive":
        return claimCount > 0 || overlayActive;
    }
  }

  /** Push `FLAG_SECURE` for the current desired level. Desktop / absent plugin:
   *  no-op (`true`). Returns whether the IPC succeeded. */
  async function pushFlag(): Promise<boolean> {
    if (!secureAvailable.value) return true; // desktop / plugin absent: no-op
    try {
      await setSecure(desiredSecure());
      return true;
    } catch {
      return false;
    }
  }

  /** Re-apply `FLAG_SECURE` for the current claim + overlay state. */
  async function applyCurrent(): Promise<boolean> {
    return pushFlag();
  }

  /**
   * Acquire one component-level claim — raises FLAG_SECURE under `"sensitive"`
   * (and is a no-op `true` under `"always"`; ignored under `"off"`, where the
   * user opted into capture). Called by `useSecureClaim` (`withClaim`/`acquire`)
   * before a secret renders. Returns whether the plugin call succeeded; `false`
   * means the caller MUST NOT render the secret — the per-op equivalent of the
   * old route-guard abort.
   */
  async function acquireClaim(): Promise<boolean> {
    claimCount++;
    return applyCurrent();
  }

  /** Release one claim (idempotent, floored at 0). Called by `useSecureClaim`
   *  when a secret is cleared or its component unmounts. */
  async function releaseClaim(): Promise<boolean> {
    if (claimCount > 0) claimCount--;
    return applyCurrent();
  }

  /**
   * Load availability + the master mode once, then reconcile. Idempotent. Call
   * from `App.vue` on mount. An absent or unrecognized backend value (e.g.
   * `"unknown"`, a forward-compat sink from a newer build) resolves to
   * `"sensitive"`.
   */
  async function initSecureScreen() {
    if (initialized) return;
    initialized = true;
    try {
      secureAvailable.value = await screenSecureAvailable();
    } catch {
      // Couldn't confirm availability. On Android this command always returns
      // `true`, so a rejection means the bridge is flaky — NOT "desktop". Assume
      // available so subsequent calls are ATTEMPTED and fail-closed (an acquire
      // that fails aborts the reveal) rather than silently no-op'd (fail-open).
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
    await applyCurrent();
  }

  /**
   * Reflect whether the global unlock overlay is up. Under `"sensitive"` the
   * overlay collects the identity passphrase, so raising it forces `FLAG_SECURE`
   * on (see `desiredSecure`) even with no claim. Re-applies immediately; returns
   * the plugin result (the `App.vue` watcher ignores it).
   */
  function setSecureOverlay(active: boolean): Promise<boolean> {
    overlayActive = active;
    return applyCurrent();
  }

  /**
   * Persist the master mode, then re-apply. Returns `false` (reverting the
   * in-memory ref and re-applying) if persistence failed, so the UI never shows
   * a mode that didn't actually save — UI/disk/window stay in sync instead of
   * desyncing on a failed write.
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
      await applyCurrent();
      return false;
    }
    await applyCurrent();
    return true;
  }

  /** Reset the one-shot latch and re-init from the backend. See interface doc. */
  async function reload() {
    initialized = false;
    await initSecureScreen();
  }

  return {
    secureScreenMode,
    secureAvailable,
    initSecureScreen,
    reload,
    setSecureOverlay,
    setSecureScreenMode,
    acquireClaim,
    releaseClaim,
  };
}

/**
 * Inject the app-wide screen-capture-protection state. Must be called within a
 * component `setup()` under a tree that provided `SECURE_SCREEN_KEY`. Throws if
 * missing so a forgotten `provide` fails loudly.
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
