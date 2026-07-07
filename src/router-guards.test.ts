// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { createSecureScreen } from "@/composables/useSecureScreen";
import { createToast } from "@/composables/useToast";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createApp, defineComponent, h, type Component } from "vue";
import { createMemoryHistory, createRouter, RouterView } from "vue-router";
import { installRouteGuards } from "./router-guards";

// setup.ts mocks vue-router for the page tests; this test needs the REAL router
// (createMemoryHistory, real navigation, lazy component resolution) to exercise
// the raise-before-paint invariant, so override the mock with the actual module.
vi.mock("vue-router", async () => await vi.importActual("vue-router"));

/**
 * Regression guard for the lazy-route + screen-capture invariant (the load-
 * bearing reason the route guards were extracted into `installRouteGuards`).
 *
 * The invariant: a `meta.secure` page is NEVER shown unprotected — not even
 * during the chunk-fetch window lazy routes open up. `beforeEach` must RAISE
 * FLAG_SECURE (awaited) before the target component mounts/paints, so a slow
 * chunk can never slip a secret page on screen unsecured. This test holds the
 * chunk's resolution in its own hand and asserts the raise still fires first.
 */
describe("installRouteGuards", () => {
  let cleanup: (() => void) | null = null;
  beforeEach(() => {
    vi.clearAllMocks();
    cleanup = null;
  });
  afterEach(() => {
    cleanup?.();
  });

  it("raises FLAG_SECURE before a lazy secret route mounts", async () => {
    // Record the order of every IPC `set_secure` and the lazy component's mount,
    // so the test can assert sequencing (raise → mount), not just occurrence.
    const order: string[] = [];
    vi.mocked(invoke).mockImplementation(((cmd: string, args?: unknown) => {
      if (cmd === "get_auth_state") {
        return Promise.resolve({ configured: true });
      }
      if (cmd === "set_secure" || cmd === "plugin:screen-secure|set_secure") {
        order.push(`set_secure:${(args as { secure: boolean }).secure}`);
        return Promise.resolve();
      }
      return Promise.resolve();
    }) as typeof invoke);

    const secureState = createSecureScreen({ available: true });
    const toastState = createToast();

    // A lazy component whose chunk resolution the test controls: the route's
    // `component: () => promise` keeps the chunk pending until `resolveLazy` is
    // called, exposing the window BETWEEN beforeEach (raise) and mount.
    let resolveLazy!: (c: Component) => void;
    const lazyPromise = new Promise<Component>((r) => {
      resolveLazy = r;
    });
    const LazySecret = defineComponent({
      setup() {
        order.push("mounted");
        return () => h("div", "secret");
      },
    });

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/", name: "home", component: { render: () => h("div") } },
        {
          path: "/secret",
          name: "secret",
          meta: { secure: true },
          component: () => lazyPromise,
        },
      ],
    });
    installRouteGuards(router, secureState, toastState);

    // The component only mounts when a <router-view> renders it, so mount a
    // minimal app. The initial "/" nav (synchronous home component) completes
    // first; the secret nav is driven below.
    const app = createApp({ render: () => h(RouterView) });
    app.use(router);
    const el = document.createElement("div");
    document.body.appendChild(el);
    app.mount(el);
    cleanup = () => app.unmount();
    await flushPromises(); // initial "/" navigation settles

    // Drop the home nav's IPC noise so the assertions are about the secret nav.
    order.length = 0;

    // Fire the secret navigation but do NOT resolve the lazy chunk yet.
    // flushPromises lets beforeEach (the raise) run while the chunk is pending.
    void router.push("/secret");
    await flushPromises();

    // The raise MUST have fired already; the chunk is still pending so the page
    // has NOT mounted — the secret route is not on screen unprotected.
    expect(order).toContain("set_secure:true");
    expect(order).not.toContain("mounted");

    // Now resolve the chunk → the component mounts, then afterEach settles.
    resolveLazy(LazySecret);
    await flushPromises();

    // The raise preceded the mount (raise-before-paint).
    expect(order).toContain("mounted");
    expect(order.indexOf("set_secure:true")).toBeLessThan(
      order.indexOf("mounted"),
    );
  });
});
