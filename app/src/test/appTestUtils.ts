// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { type RuntimePlatform } from "@/api";
import {
  STACKED_ROUTER_VIEW_KEY,
  type StackedRouterViewState,
} from "@/components/StackedRouterView.vue";
import {
  APP_LOCK_KEY,
  BACK_HANDLER_KEY,
  createAppLockStore,
  createBackHandlerRegistry,
  createDialog,
  createDraftsNotice,
  createLockState,
  createPlatform,
  createScrollLockController,
  createSecureScreen,
  createSecuritySettings,
  createToast,
  DIALOG_KEY,
  DRAFTS_NOTICE_KEY,
  LOCK_KEY,
  PLATFORM_KEY,
  SCROLL_LOCK_KEY,
  SECURE_SCREEN_KEY,
  SECURITY_SETTINGS_KEY,
  TOAST_KEY,
} from "@/composables";
import { mount, type ComponentMountingOptions } from "@vue/test-utils";
import { vi } from "vitest";
import { type Component } from "vue";

interface MountWithAppOptions<C extends Component> {
  /** Default `true`: start the lock in the "unlocked, identity cached" state page
   *  tests historically got from `__unlockForTests`. Pass `false` for tests that
   *  need the identity NOT cached (e.g. AUTH_CANCELLED / parked-on-overlay). */
  unlocked?: boolean;
  /** Default `true`: start secureScreen with the plugin reported available
   *  (Android, the production target). Pass `false` for desktop/no-plugin. */
  secureAvailable?: boolean;
  /** Default `"android"`: the platform fact for per-platform UI gating.
   *  Independent of `secureAvailable` — a desktop test passes `"linux"` (or
   *  `"macos"`/`"windows"`) explicitly so the gated UI hides. */
  platform?: RuntimePlatform;
  /** Forwarded to `mount`, merged under the app-shell provide block. */
  mountOpts?: ComponentMountingOptions<C>;
}

/**
 * A {@link StackedRouterViewState} stand-in for page tests. Production arms
 * `whenSettled` from the `<router-view>` `<Transition>`'s after-enter hook
 * (StackedRouterView.vue), which page tests don't mount — so here `whenSettled`
 * returns a Promise the test resolves by calling `releaseEnter()`. That lets a
 * deep-link test hold the page's focus at the slide-settle gate, assert nothing
 * fires yet, then release and assert it does. For pages that never call
 * `whenSettled` (no `?focus=`) it stays inert.
 */
function createTestStackedRouterView(): StackedRouterViewState & {
  releaseEnter(): void;
} {
  const pending: Array<() => void> = [];
  return {
    whenSettled: () => new Promise<void>((resolve) => pending.push(resolve)),
    releaseEnter: () => {
      pending.splice(0).forEach((resolve) => resolve());
    },
  };
}

/**
 * Mount `comp` with every app-shell state provided, fresh per call. Returns the
 * wrapper and every state handle so a test can drive any instance via real
 * methods. Providing them all every time covers transitive injection
 * automatically
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
  const draftsNotice = createDraftsNotice();
  const secureScreen = createSecureScreen({
    available: opts.secureAvailable !== false,
  });
  const platform = createPlatform({ platform: opts.platform ?? "android" });
  const securitySettings = createSecuritySettings();
  const toast = createToast();
  const dialog = createDialog();
  const scrollLock = createScrollLockController();
  const backHandler = createBackHandlerRegistry();
  const stackedRouterView = createTestStackedRouterView();
  // Page tests don't mount DialogHost, so drive `confirm` directly. Default to
  // "proceed" (the former global confirm()=true default in setup.ts); a test
  // that needs the cancel branch overrides it:
  //   vi.mocked(dialog.dialog.confirm).mockResolvedValue(false)
  vi.spyOn(dialog.dialog, "confirm").mockResolvedValue(true);
  const wrapper = mount(comp, {
    ...opts.mountOpts,
    global: {
      ...opts.mountOpts?.global,
      provide: {
        ...opts.mountOpts?.global?.provide,
        [LOCK_KEY]: lock,
        [APP_LOCK_KEY]: appLock,
        [DRAFTS_NOTICE_KEY]: draftsNotice,
        [SECURE_SCREEN_KEY]: secureScreen,
        [PLATFORM_KEY]: platform,
        [SECURITY_SETTINGS_KEY]: securitySettings,
        [TOAST_KEY]: toast,
        [DIALOG_KEY]: dialog,
        [SCROLL_LOCK_KEY]: scrollLock,
        [BACK_HANDLER_KEY]: backHandler,
        [STACKED_ROUTER_VIEW_KEY]: stackedRouterView,
      },
    },
  });
  return {
    wrapper,
    lock,
    appLock,
    draftsNotice,
    secureScreen,
    platform,
    securitySettings,
    toast,
    dialog,
    scrollLock,
    backHandler,
    stackedRouterView,
  };
}
