// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { backgroundSync, getAppConfig, type SyncOutcome } from "@/api";
import { ref, watch, type Ref } from "vue";
import type { Router } from "vue-router";

import type { AppLockStore } from "./useAppLockState";

/**
 * Best-effort foreground sync (RFC R060 Tier 1) — pull + push on app cold-start
 * and resume, so the store converges with the remote without a manual
 * pull-to-refresh.
 *
 * Non-disturbing by design (the user's locked-in philosophy): success is silent
 * (the entry list just refreshes); the only thing it ever surfaces proactively
 * is a persistent `syncAttention` status badge when a decision is needed
 * (divergence / Enforce block) — it never opens a modal or enters
 * conflict-resolution on its own. Network failure is silent.
 *
 * Runs ONLY when AutoSync is on (AutoSync off ⇒ no automatic sync of any kind).
 * Needs no identity and no WebView — sync is pure git on ciphertext, and the git
 * credentials in `repo.json` are unsealed by the auth-free master key, so it runs
 * even before the age identity is unlocked. The one hard gate is the AppLock
 * biometric launch-gate: while `appLocked` is true, `repo.json` is unreadable, so
 * it skips.
 *
 * Constructed once in `App.vue` setup (deps passed in, like `createLockActivity`)
 * — single consumer, so no provide/inject.
 */

/** Minimum gap between foreground syncs — kills refocus spam, OEM `visibilitychange`
 *  churn, and a redundant resume right after a manual pull. */
const FOREGROUND_DEBOUNCE_MS = 60_000;

/** The reactive foreground-sync state consumed by the app shell (the badge). */
export interface ForegroundSyncStore {
  /** A divergence / Enforce-block outcome awaiting the user's attention, or `null`.
   *  Set passively by a foreground sync; cleared on `engage()` (badge tap) or when a
   *  later foreground sync reconciles cleanly. Drives the persistent status badge. */
  readonly syncAttention: Readonly<Ref<SyncOutcome | null>>;
  /** Arm the resume listener + fire the cold-start sync (via the `appReady` watch).
   *  Idempotent. Call once from `App.vue` `onMounted`. */
  init: () => void;
  /** Badge-tap action: clear the attention + take the user to the entry list (where
   *  a pull-to-refresh engages the existing resolve flow). */
  engage: () => void;
  /** Tear down the resume listener. Production never calls this (App.vue is the
   *  root and lives for the app's lifetime); exposed for tests that construct
   *  their own instance. */
  dispose: () => void;
}

/**
 * Create a foreground-sync instance. Production calls this once in `App.vue` setup
 * (so the `watch` calls register in setup scope) and passes the app-lock store +
 * router; tests construct their own.
 */
export function createForegroundSyncStore(
  appLock: AppLockStore,
  router: Router,
): ForegroundSyncStore {
  const syncAttention = ref<SyncOutcome | null>(null);
  /// Single-flight: prevent overlapping foreground syncs (the backend serializes
  /// on `write_mu` anyway, but this avoids a pile-up of fire-and-forget invokes).
  let syncInFlight = false;
  /// Timestamp (ms) of the last foreground sync that actually ran (a non-null
  /// outcome). NOT updated on a null (skipped/failed) result, so a dead-network
  /// sync retries on the next resume instead of being throttled.
  let lastForegroundSyncAt = 0;
  let initialized = false;

  /**
   * Run one best-effort foreground sync if every gate passes. Gates:
   * - `appLocked` / `!appReady`: defer (repo.json unreadable behind the launch gate,
   *   or boot not reconciled).
   * - `syncInFlight`: single-flight.
   * - `< FOREGROUND_DEBOUNCE_MS` since the last real sync: throttle.
   * - AutoSync off: no automatic sync at all (the user opted out).
   * Outcome → `syncAttention` for divergence/block; silent otherwise.
   */
  async function maybeSync() {
    if (syncInFlight) return;
    if (!appLock.appReady.value || appLock.appLocked.value) return;
    if (Date.now() - lastForegroundSyncAt < FOREGROUND_DEBOUNCE_MS) return;

    // Claim the single-flight slot BEFORE the first await: two triggers that both
    // reach the getAppConfig() IPC (e.g. back-to-back OEM `visibilitychange`) must
    // not both pass the guard above and double-invoke. The outer try/finally
    // releases the slot on every exit path (autosync-off, config error, sync error).
    syncInFlight = true;
    let outcome: SyncOutcome | null = null;
    try {
      let autosync = true;
      try {
        autosync = (await getAppConfig()).autosync ?? true;
      } catch {
        return; // can't read config — don't sync blind
      }
      if (!autosync) return; // AutoSync off ⇒ no foreground sync

      try {
        outcome = await backgroundSync();
      } catch {
        outcome = null; // invoke itself rejected — treat as silent skip
      }
    } finally {
      syncInFlight = false;
    }

    if (outcome) lastForegroundSyncAt = Date.now();
    else return; // null = skipped (no repo / app-locked / autosync-off) or silent error

    // Surface only what needs a decision, passively. Never a modal.
    if (
      outcome.kind === "diverged" ||
      (outcome.kind === "fast_forwarded" && outcome.authenticity.blocked)
    ) {
      syncAttention.value = outcome;
    } else {
      // A clean fast-forward reconciled a prior divergence (if any) — clear it.
      syncAttention.value = null;
    }
  }

  // Cold-start: fire once `appReady` is reconciled (app-lock-off flips it true on
  // init; app-lock-on stays `appLocked` so this no-ops until the unlock watch
  // below fires). `immediate` covers the already-ready race.
  watch(
    () => appLock.appReady.value,
    (ready) => {
      if (ready) void maybeSync();
    },
    { immediate: true },
  );
  // Unlock after a cold-start-under-app-lock or a resume relock: sync now.
  watch(
    () => appLock.appLocked.value,
    (locked, prev) => {
      if (prev && !locked) void maybeSync();
    },
  );

  /** Resume handler: sync on return to the foreground, but only when the app-lock
   *  gate is OFF — when it's on, R058 relocks and the unlock watch owns the sync. */
  function onVisibilityChange() {
    if (document.visibilityState !== "visible") return;
    if (appLock.appLockEnabled.value) return;
    void maybeSync();
  }

  function init() {
    if (initialized) return;
    initialized = true;
    document.addEventListener("visibilitychange", onVisibilityChange);
  }

  /** Badge tap: take the user to the list (where a pull-to-refresh engages the
   *  existing resolve flow — the foreground sync itself never enters
   *  conflict-resolution). The badge is NOT cleared here: it stays until a later
   *  foreground sync reconciles cleanly (the `else` branch in `maybeSync`), so an
   *  unresolved divergence doesn't go silent while the app stays foregrounded. */
  function engage() {
    void router.push({ name: "entries" });
  }

  function dispose() {
    document.removeEventListener("visibilitychange", onVisibilityChange);
  }

  return { syncAttention, init, engage, dispose };
}
