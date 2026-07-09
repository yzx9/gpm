// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { enableAutoUnmount, flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import BaseModalShell from "./BaseModalShell.vue";

// BaseModalShell locks the document scroller on mount (useScrollLock). Unmount
// every wrapper after each test so the shared lock count returns to 0 instead of
// climbing across tests that mount without an explicit unmount.
enableAutoUnmount(afterEach);

// Override the global setup.ts no-op mock with a DEFERRED onBackButtonPress so
// tests can drive "registration completes" and "back pressed". Mirrors the
// composable's own test. unregister() clears the captured handler so fireBack()
// after unregister is a no-op (mirrors Tauri no longer emitting to a released
// listener). Applies to this file only.
const api = vi.hoisted(() => {
  let handler: ((p: { canGoBack: boolean }) => void) | null = null;
  const unregister = vi.fn(async () => {
    handler = null;
  });
  let pendingResolve: ((l: { unregister: typeof unregister }) => void) | null =
    null;
  const onBackButtonPress = vi.fn((h: (p: { canGoBack: boolean }) => void) => {
    handler = h;
    return new Promise<{ unregister: typeof unregister }>((res) => {
      pendingResolve = res;
    });
  });
  const resolveRegistration = () => {
    pendingResolve?.({ unregister });
    pendingResolve = null;
  };
  const fireBack = () => {
    handler?.({ canGoBack: false });
  };
  return { onBackButtonPress, unregister, resolveRegistration, fireBack };
});
vi.mock("@tauri-apps/api/app", () => ({
  onBackButtonPress: api.onBackButtonPress,
}));

describe("BaseModalShell", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("emits `close` when the overlay backdrop is clicked", async () => {
    const wrapper = mount(BaseModalShell, { props: { variant: "center" } });
    await wrapper.find(".overlay").trigger("click");
    expect(wrapper.emitted("close")).toHaveLength(1);
  });

  it("does NOT emit `close` when a click lands inside the card", async () => {
    const wrapper = mount(BaseModalShell, {
      props: { variant: "center" },
      slots: { default: "<p>body</p>" },
    });
    // .wrap sits between the backdrop and the card; a click there bubbles to
    // .overlay but is not `.self`, so it must not close.
    await wrapper.find(".wrap").trigger("click");
    expect(wrapper.emitted("close")).toBeUndefined();
  });

  it("defaults z-index to 60 for `center` and 40 for `sheet`", () => {
    const center = mount(BaseModalShell, { props: { variant: "center" } });
    expect(center.find(".overlay").attributes("style")).toContain(
      "z-index: 60",
    );

    const sheet = mount(BaseModalShell, { props: { variant: "sheet" } });
    expect(sheet.find(".overlay").attributes("style")).toContain("z-index: 40");
  });

  it("honors an explicit `z` override (app-lock sits above the identity modal)", () => {
    const wrapper = mount(BaseModalShell, {
      props: { variant: "center", z: 70 },
    });
    expect(wrapper.find(".overlay").attributes("style")).toContain(
      "z-index: 70",
    );
  });

  it("emits `close` on Android back by default (dismissOnBack=true)", async () => {
    const wrapper = mount(BaseModalShell, { props: { variant: "center" } });
    await flushPromises();
    api.fireBack();
    await flushPromises();
    expect(wrapper.emitted("close")).toHaveLength(1);
  });

  it("traps back when `dismissOnBack=false` — no `close`, but the listener is still registered (suppresses default goBack)", async () => {
    const wrapper = mount(BaseModalShell, {
      props: { variant: "center", dismissOnBack: false },
    });
    await flushPromises();
    api.fireBack();
    await flushPromises();
    expect(wrapper.emitted("close")).toBeUndefined();
    expect(api.onBackButtonPress).toHaveBeenCalled();
  });

  it("does NOT emit `close` on a backdrop tap when `dismissOnBackdrop=false`", async () => {
    const wrapper = mount(BaseModalShell, {
      props: { variant: "center", dismissOnBackdrop: false },
    });
    await wrapper.find(".overlay").trigger("click");
    expect(wrapper.emitted("close")).toBeUndefined();
  });

  it("back still dismisses when `dismissOnBackdrop=false` (the two props are decoupled)", async () => {
    const wrapper = mount(BaseModalShell, {
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
    const wrapper = mount(BaseModalShell, {
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
    const wrapper = mount(BaseModalShell, { props: { variant: "center" } });
    await flushPromises();
    api.resolveRegistration();
    await flushPromises();
    expect(api.unregister).not.toHaveBeenCalled();
    wrapper.unmount();
    await flushPromises();
    expect(api.unregister).toHaveBeenCalledTimes(1);
  });
});
