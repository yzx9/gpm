// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import type { InjectionKey, Ref } from "vue";
import { inject, ref } from "vue";
import { START_LOCATION, type Router } from "vue-router";

/**
 * The `<Transition :name>` for the `<router-view>` swap.
 * `""` means no animation (instant) — used on the initial paint and on
 * `router.replace` (terminal/reset flows), where position is unchanged.
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
 * Track navigation direction, exposing a reactive transition name for the
 * `<router-view>` swap.
 *
 * Direction is read by comparing the just-settled `history.state.position` to
 * the previously-settled one inside `afterEach`. This is deliberately NOT a
 * `beforeEach` capture: on popstate the history state is already the target
 * entry by the time router guards run, so a before-capture cannot tell back
 * from forward. Comparing two settled positions dodges that entirely.
 *
 * R031: screen-capture protection is component-level now, so there is no
 * secure↔capturable boundary to freeze on — every navigation animates by
 * direction. (Previously a `"sensitive"`-mode boundary forced `""` because the
 * route guard dropped FLAG_SECURE mid-slide; that guard is gone.)
 */
export function createNavDirection(router: Router): NavDirectionState {
  let lastPosition = historyPosition();
  const transitionName = ref<NavTransitionName>("");

  router.afterEach((_to, from) => {
    const pos = historyPosition();
    // The initial navigation has no real "from" route (START_LOCATION) and no
    // meaningful direction — never animate the first paint.
    if (from === START_LOCATION) {
      transitionName.value = "";
      lastPosition = pos;
      return;
    }
    if (pos > lastPosition) {
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
