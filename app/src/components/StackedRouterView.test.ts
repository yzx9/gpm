// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { createStackedRouterState } from "@/components/StackedRouterView.vue";
import { flushPromises } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { h } from "vue";
import { createMemoryHistory, createRouter } from "vue-router";

// setup.ts mocks vue-router for the page tests; this test needs the REAL router
// to exercise the afterEach gate, so override the mock with the actual module.
vi.mock("vue-router", async () => await vi.importActual("vue-router"));

const Plain = { render: () => h("div") };

/**
 * Build a router with a non-secret route (`/`) and two formerly-secure routes.
 * R031 removed the route-level secure flag, so the transition no longer freezes
 * on the secure↔capturable boundary — every navigation animates by direction.
 */
function buildRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/", name: "home", component: Plain },
      { path: "/secret", name: "secret", component: Plain },
      { path: "/other", name: "other", component: Plain },
    ],
  });
}

/**
 * The gate reads `window.history.state.position` for direction. Memory history
 * (the only kind that doesn't hang jsdom) never populates `window.history`, so
 * stub its `state` getter with a position the test controls. `goto` sets that
 * position to reflect the navigation it is about to perform, then drives it.
 */
let position = 0;
async function goto(
  router: ReturnType<typeof buildRouter>,
  path: string,
  pos: number,
) {
  position = pos;
  await router.push(path);
  await flushPromises();
}

describe("createStackedRouterState", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    position = 0;
    vi.spyOn(window.history, "state", "get").mockImplementation(
      () => ({ position }) as HistoryState,
    );
  });

  it("does not animate the initial paint", async () => {
    const router = buildRouter();
    const { transitionName } = createStackedRouterState(router);

    // The first navigation has no real "from" (START_LOCATION) ⇒ never animates.
    await goto(router, "/secret", 1);
    expect(transitionName.value).toBe("");
  });

  it("slides across the former secure↔capturable boundary", async () => {
    // R031: the boundary freeze is gone — a push from a secret page to a
    // non-secret page animates forward, where it previously froze to "".
    const router = buildRouter();
    const { transitionName } = createStackedRouterState(router);

    await goto(router, "/secret", 1); // initial paint ⇒ ""
    await goto(router, "/", 2); // secret → home: animates now
    expect(transitionName.value).toBe("slide-forward");
  });

  it("animates push/pop between like routes", async () => {
    const router = buildRouter();
    const { transitionName } = createStackedRouterState(router);

    await goto(router, "/secret", 1); // initial paint ⇒ "", current = /secret
    await goto(router, "/other", 2); // forward push animates
    expect(transitionName.value).toBe("slide-forward");

    position = 1;
    router.back();
    await flushPromises();
    expect(transitionName.value).toBe("slide-back");
  });

  it("does not animate a replace (position unchanged)", async () => {
    const router = buildRouter();
    const { transitionName } = createStackedRouterState(router);

    await goto(router, "/secret", 1); // initial paint ⇒ "", current = /secret
    position = 1; // unchanged from the previous nav
    await router.replace("/other");
    await flushPromises();
    expect(transitionName.value).toBe("");
  });

  describe("whenSettled (enter-transition settle signal)", () => {
    // The settle hooks are driven by <Transition>'s JS hooks in
    // StackedRouterView.vue; here we call them directly to pin the settle
    // contract independent of CSS. whenSettled() takes no element — the
    // component tracks the entering page internally.
    it("resolves once after-enter fires for that element", async () => {
      const nav = createStackedRouterState(buildRouter());
      const el = document.createElement("div");
      nav.onBeforeEnter(el); // <Transition> before-enter arms the entry
      let resolved = false;
      void nav.whenSettled().then(() => {
        resolved = true;
      });
      await flushPromises();
      expect(resolved).toBe(false); // slide still in flight
      nav.onAfterEnter(el); // slide ended
      await flushPromises();
      expect(resolved).toBe(true);
    });

    it("resolves immediately when no enter was armed (initial paint / query-only replace)", async () => {
      const nav = createStackedRouterState(buildRouter());
      // No onBeforeEnter (no transition ran) ⇒ nothing to wait for.
      let resolved = false;
      void nav.whenSettled().then(() => {
        resolved = true;
      });
      await flushPromises();
      expect(resolved).toBe(true);
    });

    it("a cancelled enter resolves its own awaiter, not a later page's", async () => {
      // Pins the per-element (WeakMap) design + the no-arg capture: each page
      // captures the settle promise current at its before-enter, and a cancelled
      // enter resolves THAT awaiter while the later page's stays pending.
      const nav = createStackedRouterState(buildRouter());
      const a = document.createElement("div");
      const b = document.createElement("div");
      nav.onBeforeEnter(a);
      const aSettle = nav.whenSettled();
      let aResolved = false;
      void aSettle.then(() => {
        aResolved = true;
      });
      nav.onBeforeEnter(b); // a newer navigation arms its own entry
      const bSettle = nav.whenSettled();
      let bResolved = false;
      void bSettle.then(() => {
        bResolved = true;
      });
      await flushPromises();
      expect(aResolved).toBe(false);
      expect(bResolved).toBe(false);
      nav.onEnterCancelled(a); // A interrupted, not completed
      await flushPromises();
      expect(aResolved).toBe(true);
      expect(bResolved).toBe(false);
      nav.onAfterEnter(b); // B completes normally
      await flushPromises();
      expect(bResolved).toBe(true);
    });
  });
});

/** Minimal shape of `window.history.state` the gate reads (position only). */
type HistoryState = { position: number };
