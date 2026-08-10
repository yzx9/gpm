<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<script lang="ts">
import {
  defineComponent,
  inject,
  provide,
  ref,
  type InjectionKey,
  type Ref,
} from "vue";
import { START_LOCATION, useRouter, type Router } from "vue-router";

/**
 * The `<Transition :name>` for the `<router-view>` swap.
 * `""` means no animation (instant) — used on the initial paint and on
 * `router.replace` (terminal/reset flows), where position is unchanged.
 */
export type NavTransitionName = "" | "slide-forward" | "slide-back";

/**
 * What a routed page reaches via {@link useStackedRouterView}: just the settle
 * signal. The `<Transition>`'s hooks and the slide name stay internal to this
 * component — a page has no business wiring transition hooks.
 */
export interface StackedRouterViewState {
  /**
   * Resolves when THIS page's slide-in enter transition finishes — or
   * immediately if there was no enter (initial paint, or a query-only replace
   * that didn't remount). No argument: the component already knows which page
   * is entering (from its own `<Transition>` hooks), and by the component tree a
   * page calling this in its `onMounted` IS the page currently entering, so it
   * gets its own settle promise.
   *
   * Per-page correctness under rapid navigation (a cancelled enter resolving
   * the right awaiter) is handled internally; a page that's leaving bails via
   * its own `alive`/route guards when this resolves.
   */
  whenSettled(): Promise<void>;
}

export const STACKED_ROUTER_VIEW_KEY: InjectionKey<StackedRouterViewState> =
  Symbol("StackedRouterView");

/**
 * Inject the stacked router-view's settle signal. Call from a routed page (a
 * descendant of `<StackedRouterView>`).
 */
export function useStackedRouterView(): StackedRouterViewState {
  const s = inject(STACKED_ROUTER_VIEW_KEY);
  if (!s) {
    throw new Error(
      "useStackedRouterView() requires <StackedRouterView> to be mounted above it",
    );
  }
  return s;
}

/** A pending settle for one entering page, keyed by its root element. */
interface SettleEntry {
  promise: Promise<void>;
  resolve: () => void;
}

function historyPosition(): number {
  // vue-router's cursor: 0 at the initial entry, +1 per push, preserved on
  // replace. See `nav.ts` for the field's provenance.
  return (window.history.state as { position?: number } | null)?.position ?? 0;
}

/** Internal state returned by {@link createStackedRouterState}. */
interface StackedRouterState {
  readonly transitionName: Readonly<Ref<NavTransitionName>>;
  readonly onBeforeEnter: (el: Element) => void;
  readonly onAfterEnter: (el: Element) => void;
  readonly onEnterCancelled: (el: Element) => void;
  whenSettled(): Promise<void>;
}

/**
 * Owns the `<router-view>` transition: a reactive transition name (push →
 * slide-forward, pop → slide-back, replace/initial → instant) AND an awaitable
 * signal for when an entering page's slide finishes, so deep-link
 * scroll/highlight can land AFTER the slide instead of racing it
 * (see SettingsIdentityPage).
 *
 * Direction is read by comparing the just-settled `history.state.position` to
 * the previously-settled one inside `afterEach`. This is deliberately NOT a
 * `beforeEach` capture: on popstate the history state is already the target
 * entry by the time router guards run, so a before-capture cannot tell back
 * from forward. Comparing two settled positions dodges that entirely.
 *
 * The settle signal rides the `<Transition>`'s JavaScript hooks — `after-enter`
 * IS "the slide ended", `enter-cancelled` IS "it was interrupted" — rather than
 * a hand-rolled `transitionend` listener plus a guessed/fallback duration.
 * Vue already does authoritative transition-end detection (CSS transition vs
 * animation, reduced motion, cancellation), so nothing here can drift from
 * style.css.
 *
 * Exported (not just consumed by the component) so the contract is unit-testable
 * by driving the hooks directly, without mounting a real `<Transition>` (which
 * jsdom can't run faithfully).
 *
 * R031: screen-capture protection is component-level now, so there is no
 * secure↔capturable boundary to freeze on — every navigation animates by
 * direction. (Previously a `"sensitive"`-mode boundary forced `""` because the
 * route guard dropped FLAG_SECURE mid-slide; that guard is gone.)
 */
export function createStackedRouterState(router: Router): StackedRouterState {
  let lastPosition = historyPosition();
  const transitionName = ref<NavTransitionName>("");
  // One pending settle per entering page, keyed by its root element. `before-
  // enter` arms it (before the page's onMounted, so a consumer reading
  // whenSettled() from onMounted onward always sees its own entry); `after-
  // enter`/`enter-cancelled` resolve it. Entries drop themselves when the
  // element is GC'd after leave.
  const settleByEl = new WeakMap<Element, SettleEntry>();
  // The promise a no-arg whenSettled() returns: always "the page currently
  // entering". A page calls whenSettled() in its onMounted, at which point — by
  // the component tree — it IS that page, so this is its own settle promise.
  let currentSettle: Promise<void> = Promise.resolve();

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

  return {
    transitionName,
    onBeforeEnter: (el) => {
      let resolve!: () => void;
      const promise = new Promise<void>((r) => {
        resolve = r;
      });
      settleByEl.set(el, { promise, resolve });
      currentSettle = promise;
    },
    onAfterEnter: (el) => settleByEl.get(el)?.resolve(),
    onEnterCancelled: (el) => settleByEl.get(el)?.resolve(),
    whenSettled: () => currentSettle,
  };
}

export default defineComponent({
  name: "StackedRouterView",
  setup() {
    const state = createStackedRouterState(useRouter());
    // Only the settle signal crosses the boundary into routed pages; the
    // transition name + hooks stay component-internal.
    provide(STACKED_ROUTER_VIEW_KEY, { whenSettled: state.whenSettled });
    const { transitionName, onBeforeEnter, onAfterEnter, onEnterCancelled } =
      state;
    return { transitionName, onBeforeEnter, onAfterEnter, onEnterCancelled };
  },
});
</script>

<template>
  <!--
    Stack-style slide between pages. No `mode="out-in"`: push/pop animate the
    departing and arriving pages simultaneously (iOS NavigationController
    feel). `:key="route.path"` (NOT fullPath) makes Vue treat each *page* as a
    distinct element so the transition fires on every real nav — but a
    query-only change does NOT remount: the Settings→Identity deep-link clears
    its `?focus=` query after arriving, and keying on fullPath would tear that
    page down mid-highlight (the flash lives on the arriving instance).
    `transitionName` is "" only on the initial paint and on replace navigations
    — screen-capture protection is component-level (R031), so there is no
    secure↔capturable boundary to freeze the slide on.

    The enter hooks feed `whenSettled` — a promise deep-link focus logic awaits
    so its scroll/highlight lands AFTER the slide. They ride Vue's own
    transition lifecycle (`after-enter` = "slide ended"), so there is no
    separate transitionend listener or duration to keep in sync here.
  -->
  <router-view v-slot="{ Component, route }">
    <Transition
      :name="transitionName"
      @before-enter="onBeforeEnter"
      @after-enter="onAfterEnter"
      @enter-cancelled="onEnterCancelled"
    >
      <component :is="Component" :key="route.path" />
    </Transition>
  </router-view>
</template>

<style>
/*
 * Slide classes for the <Transition :name="slide-forward|slide-back"> above.
 * Non-scoped on purpose: <Transition> applies these to the routed page's ROOT
 * element (rendered through <router-view>), not to an element this component
 * owns — and <router-view> renders no wrapper, so a scoped block would have no
 * anchor. The names match the NavTransitionName values.
 *
 * Forward (router.push) = arriving page slides in from the right, departing
 * slides out to the left. Back (router.back/pop) = the reverse. Only the
 * departing page is position:absolute during the swap; the arriving page stays
 * in flow so it carries .app-shell's height (making both absolute would collapse
 * the shell). Relies on .app-shell (nearest positioned ancestor) for the
 * absolute positioning, and on the global prefers-reduced-motion rule in
 * style.css to pin the duration to ~0.
 */
.slide-forward-enter-active,
.slide-forward-leave-active,
.slide-back-enter-active,
.slide-back-leave-active {
  transition: transform 0.25s ease;
}

.slide-forward-leave-active,
.slide-back-leave-active {
  position: absolute;
  /* Match .app-shell's safe-area padding: an absolutely-positioned child is
     offset from the padding box (the inner border edge), so it ignores the
     shell's padding-top/left/right. With `0` the departing page would jump up
     by --safe-area-inset-top the instant the transition starts — its Back
     button leaps into the cutout, then "falls" back when the in-flow arriving
     page settles. Offsetting by the same insets keeps the departing page on the
     content box it occupied in flow, so only the horizontal translateX fires. */
  top: var(--safe-area-inset-top, 0px);
  left: var(--safe-area-inset-left, 0px);
  right: var(--safe-area-inset-right, 0px);
  /* The departing page must not intercept taps during the slide — a fast
     second tap could otherwise hit a stale control (e.g. Delete) on the page
     that's sliding away. */
  pointer-events: none;
}

/* Forward: arriving from the right, departing to the left. */
.slide-forward-enter-from {
  transform: translateX(100%);
}
.slide-forward-leave-to {
  transform: translateX(-100%);
}

/* Back: arriving from the left, departing to the right. */
.slide-back-enter-from {
  transform: translateX(-100%);
}
.slide-back-leave-to {
  transform: translateX(100%);
}
</style>
