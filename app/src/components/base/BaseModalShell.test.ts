// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import {
  BACK_HANDLER_KEY,
  createBackHandlerRegistry,
  createScrollLockController,
  SCROLL_LOCK_KEY,
} from "@/composables";
import { Z } from "@/zTiers";
import {
  enableAutoUnmount,
  flushPromises,
  mount,
  type ComponentMountingOptions,
} from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import BaseModalShell from "./BaseModalShell.vue";

// BaseModalShell locks the document scroller on mount (useScrollLock). Unmount
// every wrapper after each test so the shared lock count returns to 0 instead of
// climbing across tests that mount without an explicit unmount.
enableAutoUnmount(afterEach);

// Back-handler registry: one fresh instance shared across this file's mounts
// (enableAutoUnmount drains it between tests). BaseModalShell injects it via
// BACK_HANDLER_KEY, so every mount must provide it.
const backHandler = createBackHandlerRegistry();
function mountShell(
  options: ComponentMountingOptions<typeof BaseModalShell> = {},
) {
  return mount(BaseModalShell, {
    ...options,
    global: {
      ...options.global,
      provide: {
        [SCROLL_LOCK_KEY]: createScrollLockController(),
        [BACK_HANDLER_KEY]: backHandler,
      },
    },
  });
}

// Override the global setup.ts no-op mock so tests can drive "back pressed".
// Resolves immediately — no deferred registration here (the registry's own suite
// covers the stale-registration race; deferring would leave the shared
// defaultRegistry's `subscribing` flag stuck true across tests). unregister()
// clears the captured handler so fireBack() after unregister is a no-op. This
// file only.
const api = vi.hoisted(() => {
  let handler: ((p: { canGoBack: boolean }) => void) | null = null;
  const unregister = vi.fn(async () => {
    handler = null;
  });
  const onBackButtonPress = vi.fn((h: (p: { canGoBack: boolean }) => void) => {
    handler = h;
    return Promise.resolve({ unregister });
  });
  const fireBack = () => {
    handler?.({ canGoBack: false });
  };
  return { onBackButtonPress, unregister, fireBack };
});
vi.mock("@tauri-apps/api/app", () => ({
  onBackButtonPress: api.onBackButtonPress,
}));

describe("BaseModalShell", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("emits `close` when the overlay backdrop is clicked", async () => {
    const wrapper = mountShell({ props: { variant: "center" } });
    await wrapper.find(".overlay").trigger("click");
    expect(wrapper.emitted("close")).toHaveLength(1);
  });

  it("does NOT emit `close` when a click lands inside the card", async () => {
    const wrapper = mountShell({
      props: { variant: "center" },
      slots: { default: "<p>body</p>" },
    });
    // .wrap sits between the backdrop and the card; a click there bubbles to
    // .overlay but is not `.self`, so it must not close.
    await wrapper.find(".wrap").trigger("click");
    expect(wrapper.emitted("close")).toBeUndefined();
  });

  it("defaults z-index to Z.overlay (1000) for both variants", () => {
    const center = mountShell({ props: { variant: "center" } });
    expect(center.find(".overlay").attributes("style")).toContain(
      "z-index: 1000",
    );

    const sheet = mountShell({ props: { variant: "sheet" } });
    expect(sheet.find(".overlay").attributes("style")).toContain(
      "z-index: 1000",
    );
  });

  it("honors an explicit `z` override (app-lock sits above the identity modal)", () => {
    const wrapper = mountShell({
      props: { variant: "center", z: 70 },
    });
    expect(wrapper.find(".overlay").attributes("style")).toContain(
      "z-index: 70",
    );
  });

  it("emits `close` on Android back by default (dismissOnBack=true)", async () => {
    const wrapper = mountShell({ props: { variant: "center" } });
    await flushPromises();
    api.fireBack();
    await flushPromises();
    expect(wrapper.emitted("close")).toHaveLength(1);
  });

  it("traps back when `dismissOnBack=false` — no `close`, but the listener is still registered (suppresses default goBack)", async () => {
    const wrapper = mountShell({
      props: { variant: "center", dismissOnBack: false },
    });
    await flushPromises();
    api.fireBack();
    await flushPromises();
    expect(wrapper.emitted("close")).toBeUndefined();
    expect(api.onBackButtonPress).toHaveBeenCalled();
  });

  it("does NOT emit `close` on a backdrop tap when `dismissOnBackdrop=false`", async () => {
    const wrapper = mountShell({
      props: { variant: "center", dismissOnBackdrop: false },
    });
    await wrapper.find(".overlay").trigger("click");
    expect(wrapper.emitted("close")).toBeUndefined();
  });

  it("back still dismisses when `dismissOnBackdrop=false` (the two props are decoupled)", async () => {
    const wrapper = mountShell({
      props: { variant: "center", dismissOnBackdrop: false },
    });
    await wrapper.find(".overlay").trigger("click");
    expect(wrapper.emitted("close")).toBeUndefined();
    await flushPromises();
    api.fireBack();
    await flushPromises();
    expect(wrapper.emitted("close")).toHaveLength(1);
  });

  it("respects `dismissOnBack` toggled after mount (DivergenceModal step1→step2 pattern)", async () => {
    const wrapper = mountShell({
      props: { variant: "center", dismissOnBack: true },
    });
    await flushPromises();
    api.fireBack();
    await flushPromises();
    expect(wrapper.emitted("close")).toHaveLength(1);
    await wrapper.setProps({ dismissOnBack: false });
    api.fireBack();
    await flushPromises();
    // Still 1 — the second back is trapped, no additional close.
    expect(wrapper.emitted("close")).toHaveLength(1);
  });

  it("unregisters the back listener on unmount", async () => {
    const wrapper = mountShell({ props: { variant: "center" } });
    await flushPromises();
    await flushPromises();
    expect(api.unregister).not.toHaveBeenCalled();
    wrapper.unmount();
    await flushPromises();
    expect(api.unregister).toHaveBeenCalledTimes(1);
  });

  it("two stacked shells: a higher-z shell receives back, the lower does not", async () => {
    const lower = mountShell({ props: { variant: "center" } });
    const higher = mountShell({
      props: { variant: "center", z: Z.gate },
    });
    await flushPromises();
    api.fireBack();
    await flushPromises();
    expect(higher.emitted("close")).toHaveLength(1);
    expect(lower.emitted("close")).toBeUndefined();
  });

  it("two same-z shells: the most-recently-mounted receives back (LIFO tie-break)", async () => {
    const first = mountShell({ props: { variant: "center" } });
    const second = mountShell({ props: { variant: "center" } });
    await flushPromises();
    api.fireBack();
    await flushPromises();
    expect(second.emitted("close")).toHaveLength(1);
    expect(first.emitted("close")).toBeUndefined();
  });
});
