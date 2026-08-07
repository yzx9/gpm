// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { currentLocale, i18n } from "@/i18n";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createApp, h } from "vue";
import { createMemoryHistory, createRouter, RouterView } from "vue-router";
import { installRouteGuards } from "./router-guards";

// setup.ts mocks vue-router for the page tests; this test needs the REAL router
// (createMemoryHistory, real navigation) to exercise the guard, so override the
// mock with the actual module.
vi.mock("vue-router", async () => await vi.importActual("vue-router"));

describe("installRouteGuards", () => {
  let cleanup: (() => void) | null = null;
  beforeEach(() => {
    vi.clearAllMocks();
    cleanup = null;
  });
  afterEach(() => {
    cleanup?.();
  });

  it("redirects to setup when the repo is not configured", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_auth_state")
        return Promise.resolve({ configured: false });
      return Promise.resolve();
    });

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/", name: "home", component: { render: () => h("div") } },
        {
          path: "/setup",
          name: "setup",
          component: { render: () => h("div") },
        },
      ],
    });
    installRouteGuards(router);

    const app = createApp({ render: () => h(RouterView) });
    app.use(router);
    const el = document.createElement("div");
    document.body.appendChild(el);
    app.mount(el);
    cleanup = () => app.unmount();
    await flushPromises();

    expect(router.currentRoute.value.name).toBe("setup");
  });

  it("does not touch FLAG_SECURE on navigation (protection is component-level now)", async () => {
    // R031: the guard no longer raises/settles FLAG_SECURE — that lives in
    // `useSecureClaim`. Navigating to a secret-bearing route must NOT issue a
    // set_secure IPC from the guard.
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_auth_state")
        return Promise.resolve({ configured: true });
      return Promise.resolve();
    });

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/", name: "home", component: { render: () => h("div") } },
        {
          path: "/secret",
          name: "secret",
          component: { render: () => h("div") },
        },
      ],
    });
    installRouteGuards(router);

    const app = createApp({ render: () => h(RouterView) });
    app.use(router);
    const el = document.createElement("div");
    document.body.appendChild(el);
    app.mount(el);
    cleanup = () => app.unmount();
    await flushPromises();

    await router.push("/secret");
    await flushPromises();

    const setSecureCalls = vi
      .mocked(invoke)
      .mock.calls.filter((c) => c[0] === "plugin:screen-secure|set_secure");
    expect(setSecureCalls).toHaveLength(0);
  });

  it("loads a route's meta.bundle namespace instead of its name", async () => {
    // The settings sub-pages are named `settingsGeneral` etc. but share the
    // `settings` message bundle. A naive `loadBundle(locale, to.name)` would
    // look for a non-existent `settingsGeneral.json` and leave the page
    // untranslated on a direct deep-link. `meta.bundle` overrides the name.
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_auth_state")
        return Promise.resolve({ configured: true });
      return Promise.resolve();
    });

    const locale = currentLocale();

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/", name: "home", component: { render: () => h("div") } },
        {
          path: "/sub",
          name: "settingsGeneral",
          meta: { bundle: "settings" },
          component: { render: () => h("div") },
        },
      ],
    });
    installRouteGuards(router);

    const app = createApp({ render: () => h(RouterView) });
    app.use(router);
    const el = document.createElement("div");
    document.body.appendChild(el);
    app.mount(el);
    cleanup = () => app.unmount();
    await flushPromises();

    // Start clean so the assertion proves THIS navigation loaded the bundle:
    // i18n.global is a singleton shared across test files, so a prior test may
    // have already merged `settings`.
    const fresh = i18n.global.getLocaleMessage(locale) as Record<
      string,
      unknown
    >;
    delete fresh.settings;

    await router.push("/sub");
    // loadBundle is fire-and-forget; its cold dynamic import settles on the
    // module graph's async queue and can take a variable number of event-loop
    // turns (worse under a loaded runner). Wait for the namespace against a real
    // timeout instead of a fixed flushPromises tick count, which flakes when the
    // count is too small. Presence of `settings` proves the guard loaded
    // `meta.bundle` ("settings"), not the route name (for which no bundle ships).
    await vi.waitFor(() => {
      expect(
        (i18n.global.getLocaleMessage(locale) as Record<string, unknown>)
          .settings,
      ).toBeDefined();
    });
  });

  it("logs each confirmed navigation as [nav] <name> (not fullPath)", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_auth_state")
        return Promise.resolve({ configured: true });
      return Promise.resolve();
    });
    // The console shim isn't armed in tests, so spy on console.info directly to
    // prove the afterEach fires with the route NAME only (no fullPath payload).
    const infoSpy = vi.spyOn(console, "info").mockImplementation(() => {});

    const router = createRouter({
      history: createMemoryHistory(),
      routes: [
        { path: "/", name: "home", component: { render: () => h("div") } },
        {
          path: "/entries",
          name: "entries",
          component: { render: () => h("div") },
        },
      ],
    });
    installRouteGuards(router);

    const app = createApp({ render: () => h(RouterView) });
    app.use(router);
    const el = document.createElement("div");
    document.body.appendChild(el);
    app.mount(el);
    cleanup = () => app.unmount();
    await flushPromises();

    await router.push("/entries");
    await flushPromises();

    expect(infoSpy).toHaveBeenCalledWith("[nav]", "entries");
    // Never the fullPath — it carries the full entry path on /entry/:pathMatch.
    for (const call of infoSpy.mock.calls) {
      expect(call).toHaveLength(2);
      expect(typeof call[1]).toBe("string");
    }
    infoSpy.mockRestore();
  });
});
