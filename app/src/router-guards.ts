// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { getAuthState } from "@/api";
import { currentLocale, loadBundle } from "@/i18n";
import type { Router } from "vue-router";

/**
 * Install the navigation guard that enforces configured-only access and loads
 * the arriving route's i18n bundle. Extracted from the app entry so the guard
 * is unit-testable (see `router-guards.test.ts`).
 *
 * Screen-capture protection (FLAG_SECURE) is NO LONGER route-level (R031): each
 * secret-bearing component acquires a claim while its secret is on screen
 * (`useSecureClaim`), so there is nothing for a route guard to raise/settle
 * here, and the secure↔capturable boundary no longer freezes the transition
 * (`useNavDirection` animates every navigation). The locked state is enforced
 * by the global `UnlockModal` overlay (driven by `useLockState`), not by a
 * route redirect, so the user re-authenticates in place.
 */
export function installRouteGuards(router: Router): void {
  router.beforeEach(async (to) => {
    if (to.name !== "setup") {
      try {
        const auth = await getAuthState();
        if (!auth.configured) return { name: "setup" }; // /setup leg reconciles
      } catch {
        return { name: "setup" };
      }
    }

    // Load the arriving route's message bundle for the current locale, alongside
    // the (lazy) component chunk. Fire-and-forget — a late bundle just re-renders
    // with `fallbackLocale` covering the gap. Never throws (loadBundle swallows a
    // missing bundle), so it can't block or abort the navigation. The namespace
    // defaults to the route name; a route may override it via `meta.bundle` when
    // its strings live under a different namespace (e.g. the settings sub-pages
    // share the `settings` bundle).
    const ns =
      (typeof to.meta?.bundle === "string" && to.meta.bundle) || to.name;
    if (typeof ns === "string") {
      void loadBundle(currentLocale(), ns);
    }
    return true;
  });
}
