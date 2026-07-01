// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import { defineComponent, ref, type Ref } from "vue";
import { useOverlayBackHandler } from "./useOverlayBackHandler";

// Override the global setup.ts mock with a DEFERRED onBackButtonPress so tests
// can control when "registration completes" — needed to exercise the
// toggle-off-during-await race. unregister() clears the captured handler so
// fireBack() after unregister is a no-op (mirrors Tauri no longer emitting to a
// released listener).
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

// `shown` is shared, but each test mounts a fresh component and unmounts it in
// afterEach — the composable's watcher is tied to the component scope, so an
// unmount stops it and prevents watcher pile-up across tests.
const shown: Ref<boolean> = ref(false);
const onBack = vi.fn();

const Wrapper = defineComponent({
  setup() {
    useOverlayBackHandler(shown, onBack);
    return {};
  },
  template: "<div />",
});

describe("useOverlayBackHandler", () => {
  let wrapper: ReturnType<typeof mount> | null = null;

  beforeEach(() => {
    shown.value = false;
    vi.clearAllMocks();
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = null;
  });

  const mountWrapper = () => {
    wrapper = mount(Wrapper);
    return wrapper;
  };

  it("does not register while hidden; registers when shown; fires onBack on back; unregisters when hidden again", async () => {
    mountWrapper();
    await flushPromises();
    expect(api.onBackButtonPress).not.toHaveBeenCalled();

    shown.value = true;
    await flushPromises(); // watcher fires → onBackButtonPress called, pending
    expect(api.onBackButtonPress).toHaveBeenCalledTimes(1);

    api.resolveRegistration(); // registration completes
    await flushPromises();
    expect(api.unregister).not.toHaveBeenCalled(); // still registered

    api.fireBack();
    expect(onBack).toHaveBeenCalledTimes(1);

    shown.value = false;
    await flushPromises();
    expect(api.unregister).toHaveBeenCalledTimes(1);
  });

  it("toggled off during the registration await drops the stale listener (race guard)", async () => {
    mountWrapper();
    shown.value = true;
    await flushPromises(); // onBackButtonPress called, registration pending
    expect(api.onBackButtonPress).toHaveBeenCalledTimes(1);

    shown.value = false; // hidden BEFORE registration resolves
    await flushPromises(); // else-branch: listener still null → no-op here
    expect(api.unregister).not.toHaveBeenCalled();

    api.resolveRegistration(); // stale registration now completes
    await flushPromises();
    // The guard saw `shown` already false → unregistered the stale listener…
    expect(api.unregister).toHaveBeenCalledTimes(1);
    // …and a back press no longer reaches onBack (handler cleared on unregister).
    api.fireBack();
    expect(onBack).not.toHaveBeenCalled();
  });

  it("releases the listener on unmount while the overlay is still shown (no leak)", async () => {
    mountWrapper();
    shown.value = true;
    await flushPromises();
    api.resolveRegistration(); // registration completes → listener assigned
    await flushPromises();
    expect(api.onBackButtonPress).toHaveBeenCalledTimes(1);

    // Unmount WITHOUT toggling shown back to false — exercises onBeforeUnmount.
    wrapper!.unmount();
    await flushPromises();
    expect(api.unregister).toHaveBeenCalledTimes(1);
    api.fireBack();
    expect(onBack).not.toHaveBeenCalled();
  });
});
