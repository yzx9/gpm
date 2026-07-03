// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { mount, type ComponentMountingOptions } from "@vue/test-utils";
import type { Component } from "vue";
import {
  APP_LOCK_KEY,
  createAppLockStore,
  createLockState,
  createSecureScreen,
  createSecuritySettings,
  createToast,
  LOCK_KEY,
  SECURE_SCREEN_KEY,
  SECURITY_SETTINGS_KEY,
  TOAST_KEY,
} from "@/composables";

interface MountWithAppOptions<C extends Component> {
  /** Default `true`: start the lock in the "unlocked, identity cached" state page
   *  tests historically got from `__unlockForTests`. Pass `false` for tests that
   *  need the identity NOT cached (e.g. AUTH_CANCELLED / parked-on-overlay). */
  unlocked?: boolean;
  /** Default `true`: start secureScreen with the plugin reported available
   *  (Android, the production target). Pass `false` for desktop/no-plugin. */
  secureAvailable?: boolean;
  /** Forwarded to `mount`, merged under the 5-key provide block. */
  mountOpts?: ComponentMountingOptions<C>;
}

/**
 * Mount `comp` with ALL 5 app-shell states provided, fresh per call. Returns the
 * wrapper and every state handle so a test can drive any instance via real
 * methods. Providing all 5 every time covers transitive injection automatically
 * — e.g. `EntryDetailPage` calls `useSecretReveal()` unconditionally at
 * setup, which injects `useSecuritySettings()` + `useLockState()`, so every
 * CreatePage/EntryDetailPage test needs those keys or setup throws. Fail-loud
 * (`inject` + throw) catches any forgotten key immediately.
 */
export function mountWithApp<C extends Component>(
  comp: C,
  opts: MountWithAppOptions<C> = {},
) {
  const lock = createLockState({ unlocked: opts.unlocked !== false });
  const appLock = createAppLockStore();
  const secureScreen = createSecureScreen({
    available: opts.secureAvailable !== false,
  });
  const securitySettings = createSecuritySettings();
  const toast = createToast();
  const wrapper = mount(comp, {
    ...opts.mountOpts,
    global: {
      ...opts.mountOpts?.global,
      provide: {
        ...opts.mountOpts?.global?.provide,
        [LOCK_KEY]: lock,
        [APP_LOCK_KEY]: appLock,
        [SECURE_SCREEN_KEY]: secureScreen,
        [SECURITY_SETTINGS_KEY]: securitySettings,
        [TOAST_KEY]: toast,
      },
    },
  });
  return { wrapper, lock, appLock, secureScreen, securitySettings, toast };
}
