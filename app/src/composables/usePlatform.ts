// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { runtimePlatform, type RuntimePlatform } from "@/api";
import { inject, ref, type InjectionKey, type Ref } from "vue";

/**
 * The platform gpm runs on — a general fact for UI gating, distinct from the
 * screen-secure availability probe (`useSecureScreen`). Resolved once from the
 * backend `runtime_platform()` command.
 *
 * `platform` starts `"unknown"` (Vue mounts children before `App.vue`'s init
 * runs) and resolves to a concrete value once `initPlatform` completes. On a
 * rejection it stays `"unknown"` — no fail-closed claim either way: features opt
 * IN per platform (`=== "android"`), so `"unknown"` activates nothing, the safe
 * neutral (and it avoids a boot-time flash of platform-specific UI on any
 * platform).
 *
 * Provided app-wide via `PLATFORM_KEY` (see `main.ts`); tests construct their
 * own via `createPlatform()`.
 */

/** Reactive platform fact. */
export interface PlatformState {
  /** The platform; `"unknown"` until init resolves or on rejection. Mutable so
   *  tests can seed it without invoking init. */
  platform: Ref<RuntimePlatform>;
  /** Resolve the platform from the backend once. Idempotent. */
  initPlatform: () => Promise<void>;
}

/** Seed options for `createPlatform` (test/seed only; production passes none). */
export interface CreatePlatformOptions {
  /** Start with a specific platform (default `"unknown"`). */
  platform?: RuntimePlatform;
}

/** Injection key for the app-wide platform fact. */
export const PLATFORM_KEY: InjectionKey<PlatformState> = Symbol("Platform");

/**
 * Create a fresh platform instance. Production calls this once in `main.ts` and
 * provides it; tests call it per-case for isolation.
 */
export function createPlatform(
  opts: CreatePlatformOptions = {},
): PlatformState {
  const platform = ref<RuntimePlatform>(opts.platform ?? "unknown");
  let initialized = false;

  /** Resolve the platform from the backend once. Idempotent. On rejection leave
   *  the default `"unknown"` (no platform feature activates). */
  async function initPlatform() {
    if (initialized) return;
    initialized = true;
    try {
      platform.value = await runtimePlatform();
    } catch {
      // Bridge broken (the command is a sync cfg!, so a rejection means a dead
      // IPC, not a normal flake). Leave "unknown" — no platform feature turns on.
    }
  }

  return { platform, initPlatform };
}

/**
 * Inject the app-wide platform fact. Must be called within a component
 * `setup()` under a tree that provided `PLATFORM_KEY`. Throws if missing so a
 * forgotten `provide` fails loudly.
 */
export function usePlatform(): PlatformState {
  const s = inject(PLATFORM_KEY);
  if (!s) {
    throw new Error("usePlatform() requires PLATFORM_KEY to be provided");
  }
  return s;
}
