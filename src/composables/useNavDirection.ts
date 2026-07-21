// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import type { InjectionKey, Ref } from "vue";
import { inject, ref } from "vue";
import { START_LOCATION, type Router } from "vue-router";
import type { SecureScreenState } from "./useSecureScreen";

/**
 * The `<Transition :name>` for the `<router-view>` swap.
 * `""` means no animation (instant) — used on secure↔non-secure boundaries
 * (only while screen-capture protection is active) and on `router.replace`
 * (terminal/reset flows), where position is unchanged.
 */
export type NavTransitionName = "" | "slide-forward" | "slide-back";

export interface NavDirectionState {
  readonly transitionName: Readonly<Ref<NavTransitionName>>;
}

export const NAV_DIRECTION_KEY: InjectionKey<NavDirectionState> =
  Symbol("NavDirection");

function historyPosition(): number {
  // vue-router's cursor: 0 at the initial entry, +1 per push, preserved on
  // replace. See `nav.ts` for the field's provenance.
  return (window.history.state as { position?: number } | null)?.position ?? 0;
}

/**
 * Track navigation direction and the secure-screen boundary, exposing a
 * reactive transition name for the `<router-view>` swap.
 *
 * Direction is read by comparing the just-settled `history.state.position` to
 * the previously-settled one inside `afterEach`. This is deliberately NOT a
 * `beforeEach` capture: on popstate the history state is already the target
 * entry by the time router guards run, so a before-capture cannot tell back
 * from forward. Comparing two settled positions dodges that entirely.
 *
 * Secure↔non-secure transitions are forced to `""` (no animation) ONLY in
 * `"sensitive"` mode (when `secureAvailable`): a simultaneous slide there would
 * leave the departing secure page visible while the secure-screen guard (in
 * `main.ts`) clears `FLAG_SECURE` for the arriving non-secure route — a capture
 * window. Under `"off"` / `"always"` (or on desktop, where `FLAG_SECURE` is
 * never set, or is constant) the window flag does not toggle across routes, so
 * there is no boundary to freeze on and every navigation animates for a
 * consistent feel. Like-to-like swaps carry no secure-screen concern and
 * animate freely.
 */
export function createNavDirection(
  router: Router,
  secureState: SecureScreenState,
): NavDirectionState {
  let lastPosition = historyPosition();
  const transitionName = ref<NavTransitionName>("");

  router.afterEach((to, from) => {
    const pos = historyPosition();
    // The initial navigation has no real "from" route (START_LOCATION) and no
    // meaningful direction — never animate the first paint.
    if (from === START_LOCATION) {
      transitionName.value = "";
      lastPosition = pos;
      return;
    }
    // Only the realistic capture window (protection active) freezes the slide.
    // FLAG_SECURE only toggles between routes in `"sensitive"` mode (a secret
    // route vs. the capturable list); under `"off"` / `"always"` it is constant,
    // so there's no boundary — animate every navigation for consistency. On
    // desktop `secureAvailable` is false, so the same holds. Pre-`initSecureScreen`
    // `secureAvailable` is briefly false on Android; the initial nav never
    // animates, and the `MainActivity.onCreate` secure-boot default holds
    // FLAG_SECURE on until then — so no window opens early.
    const protectionActive =
      secureState.secureAvailable.value &&
      secureState.secureScreenMode.value === "sensitive";
    const crossesSecure =
      protectionActive && !!from.meta?.secure !== !!to.meta?.secure;
    if (crossesSecure) {
      transitionName.value = "";
    } else if (pos > lastPosition) {
      transitionName.value = "slide-forward";
    } else if (pos < lastPosition) {
      transitionName.value = "slide-back";
    } else {
      transitionName.value = "";
    }
    lastPosition = pos;
  });

  return { transitionName };
}

export function useNavDirection(): NavDirectionState {
  const s = inject(NAV_DIRECTION_KEY);
  if (!s) {
    throw new Error(
      "useNavDirection() requires NAV_DIRECTION_KEY to be provided",
    );
  }
  return s;
}
