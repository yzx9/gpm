// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { getAuthState } from "@/api";
import type { SecureScreenState } from "@/composables/useSecureScreen";
import { createToast } from "@/composables/useToast";
import { currentLocale, i18n, loadBundle } from "@/i18n";
import { nextTick } from "vue";
import type { Router } from "vue-router";

/** The toast-state shape the secure-screen failure toast is shown through. */
type ToastState = ReturnType<typeof createToast>;

/**
 * Install the two navigation guards that enforce configured-only access and
 * per-route screen-capture protection. Extracted from the app entry so the
 * raise-before-paint invariant is unit-testable (see `router-guards.test.ts`).
 *
 * Screen-capture is split across the two hooks so a secret page is NEVER shown
 * unprotected — not on arrival, and not while departing:
 *  - `beforeEach` RAISES FLAG_SECURE to cover BOTH the route being left and the
 *    one entered (awaited before the target page mounts/paints). A secret route
 *    we failed to secure is never rendered — the nav aborts and a global toast
 *    explains why. With lazy routes the raise still precedes the paint: it runs
 *    here in `beforeEach`, before the component chunk even resolves.
 *  - `afterEach` SETTLES the flag to the arriving route's own level AFTER the
 *    new page has painted (`nextTick`), so clearing the flag never happens while
 *    a departing secret page is still on screen. Lazy routes extend the time
 *    between raise and settle but not the ordering — the `secureGen` token drops
 *    any settle superseded by a newer navigation.
 *
 * Register secure-screen guards before any other `afterEach` (e.g. nav
 * direction) so the settle runs first alongside them.
 */
export function installRouteGuards(
  router: Router,
  secureState: SecureScreenState,
  toastState: ToastState,
): void {
  // Navigation guard: configured-only access + per-route screen-capture
  // protection. The locked state is enforced by the global `UnlockModal` overlay
  // (driven by `useLockState`), not by a route redirect, so the user
  // re-authenticates in place instead of being navigated off their page.
  router.beforeEach(async (to, from) => {
    if (to.name !== "setup") {
      try {
        const auth = await getAuthState();
        if (!auth.configured) return { name: "setup" }; // /setup leg reconciles
      } catch {
        return { name: "setup" };
      }
    }

    // Cover the departing page too during the swap. `from.meta` is absent on the
    // initial navigation, so the first nav covers only the arriving route.
    const cover = !!(to.meta?.secure || from.meta?.secure);
    const secureOk = await secureState.raiseSecureForRoute(cover);
    if (to.meta?.secure && !secureOk) {
      toastState.toast.danger(i18n.global.t("common.toast.secureScreenFailed"));
      return false;
    }
    // Load the arriving route's message bundle for the current locale, alongside
    // the (lazy) component chunk. Fire-and-forget — securing above is what gates
    // a secret page's paint, not this bundle load, and a late bundle just
    // re-renders with `fallbackLocale` covering the gap. Never throws (loadBundle
    // swallows a missing bundle), so it can't block or abort the navigation. The
    // namespace defaults to the route name; a route may override it via
    // `meta.bundle` when its strings live under a different namespace (e.g. the
    // settings sub-pages share the `settings` bundle).
    const ns =
      (typeof to.meta?.bundle === "string" && to.meta.bundle) || to.name;
    if (typeof ns === "string") {
      void loadBundle(currentLocale(), ns);
    }
    return true;
  });

  // `afterEach` is fire-and-forget in vue-router, and it runs even for
  // navigations that aborted or were cancelled (with `failure` set). Two guards
  // keep the settle honest:
  //  - `failure`: a nav that never confirmed never mounted its target page, so
  //    settling to its route level would desync `currentRouteSecure` from the
  //    page actually on screen (and re-raise FLAG for a route we never reached).
  //  - the generation token: a newer navigation can confirm while an older one
  //    is still awaiting `nextTick`; a stale settle could otherwise drop
  //    FLAG_SECURE for a route we've already left — and the current route by
  //    then may be a secret one. Dropping superseded settles means correctness
  //    no longer depends on microtask/IPC ordering.
  let secureGen = 0;
  router.afterEach(async (to, _from, failure) => {
    // The arriving page has mounted/painted; now drop the transition-time cover
    // and reconcile to the arriving route's own level.
    const gen = ++secureGen;
    await nextTick();
    if (failure || gen !== secureGen) return; // aborted, or superseded by a newer nav
    await secureState.applySecureForRoute(!!to.meta?.secure);
  });
}
