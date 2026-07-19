// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { createNavDirection } from "@/composables/useNavDirection";
import { createSecureScreen } from "@/composables/useSecureScreen";
import { flushPromises } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { h } from "vue";
import { createMemoryHistory, createRouter } from "vue-router";

// setup.ts mocks vue-router for the page tests; this test needs the REAL router
// to exercise the afterEach gate, so override the mock with the actual module.
vi.mock("vue-router", async () => await vi.importActual("vue-router"));

const Plain = { render: () => h("div") };

/**
 * Build a router with one non-secure route (`/`) and two secure routes, so the
 * boundary the gate cares about (secure↔non-secure) is reachable in one step.
 */
function buildRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/", name: "home", component: Plain }, // NOT secure
      {
        path: "/secret",
        name: "secret",
        meta: { secure: true },
        component: Plain,
      },
      {
        path: "/other",
        name: "other",
        meta: { secure: true },
        component: Plain,
      },
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

describe("createNavDirection", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    position = 0;
    vi.spyOn(window.history, "state", "get").mockImplementation(
      () => ({ position }) as HistoryState,
    );
  });

  it("does not animate the initial paint", async () => {
    const secureState = createSecureScreen({ available: true });
    const router = buildRouter();
    const { transitionName } = createNavDirection(router, secureState);

    // The first navigation has no real "from" (START_LOCATION) ⇒ never animates.
    await goto(router, "/secret", 1);
    expect(transitionName.value).toBe("");
  });

  it("freezes the transition on a secure boundary while protection is on", async () => {
    const secureState = createSecureScreen({ available: true }); // toggle ON
    const router = buildRouter();
    const { transitionName } = createNavDirection(router, secureState);

    await goto(router, "/secret", 1); // initial paint ⇒ ""
    // secret (secure) → home (non-secure): boundary crossed ⇒ no animation,
    // even though the push advanced the history cursor.
    await goto(router, "/", 2);
    expect(transitionName.value).toBe("");
  });

  it("slides across a secure boundary when the master toggle is off", async () => {
    const secureState = createSecureScreen({ available: true });
    secureState.secureScreen.value = false; // user disabled screen-capture protection
    const router = buildRouter();
    const { transitionName } = createNavDirection(router, secureState);

    await goto(router, "/secret", 1); // initial paint ⇒ ""
    // Same secure→non-secure boundary as above, but protection is off so
    // FLAG_SECURE never toggles between routes — the slide animates normally.
    await goto(router, "/", 2);
    expect(transitionName.value).toBe("slide-forward");
  });

  it("slides across a secure boundary on desktop (protection unavailable)", async () => {
    // Desktop: the screen-secure plugin is absent, so FLAG_SECURE is never set
    // on any route — the boundary freeze must not apply even with the toggle on.
    const secureState = createSecureScreen(); // available defaults false (desktop)
    const router = buildRouter();
    const { transitionName } = createNavDirection(router, secureState);

    await goto(router, "/secret", 1); // initial paint ⇒ ""
    await goto(router, "/", 2); // secure→non-secure boundary, no plugin ⇒ slide
    expect(transitionName.value).toBe("slide-forward");
  });

  it("animates push/pop between like-protection routes", async () => {
    const secureState = createSecureScreen({ available: true });
    const router = buildRouter();
    const { transitionName } = createNavDirection(router, secureState);

    await goto(router, "/secret", 1); // initial paint ⇒ "", current = /secret
    // secret → other (both secure): forward push animates.
    await goto(router, "/other", 2);
    expect(transitionName.value).toBe("slide-forward");

    // pop back to secret: backward navigation animates.
    position = 1;
    router.back();
    await flushPromises();
    expect(transitionName.value).toBe("slide-back");
  });

  it("does not animate a replace (position unchanged)", async () => {
    const secureState = createSecureScreen({ available: true });
    const router = buildRouter();
    const { transitionName } = createNavDirection(router, secureState);

    await goto(router, "/secret", 1); // initial paint ⇒ "", current = /secret
    // replace preserves history position ⇒ no animation, even though secret →
    // other would otherwise be a like-protection forward push.
    position = 1; // unchanged from the previous nav
    await router.replace("/other");
    await flushPromises();
    expect(transitionName.value).toBe("");
  });
});

/** Minimal shape of `window.history.state` the gate reads (position only). */
type HistoryState = { position: number };
