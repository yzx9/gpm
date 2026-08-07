// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { Z } from "@/zTiers";
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { defineComponent, ref, type Ref } from "vue";
import {
  BACK_HANDLER_KEY,
  type BackHandlerHandle,
  type BackHandlerRegistry,
} from "./useBackHandlerRegistry";
import { useOverlayBackHandler } from "./useOverlayBackHandler";

// A fake registry isolates the composable from the real one (which has its own
// suite). We only assert push/pop wiring + z forwarding — the dispatch rule,
// listener lifecycle, and stale-registration guards live in
// useBackHandlerRegistry.test.ts. (The old per-instance "toggled-off-during-await"
// race test is gone: that guard moved to the registry.) The fake is PROVIDED via
// BACK_HANDLER_KEY — the composable injects it (no registry parameter).
const HANDLE = 1 as unknown as BackHandlerHandle;
function makeFakeRegistry(): BackHandlerRegistry {
  return { push: vi.fn(() => HANDLE), pop: vi.fn() };
}

const shown: Ref<boolean> = ref(false);
const onBack = vi.fn();

describe("useOverlayBackHandler", () => {
  beforeEach(() => {
    shown.value = false;
    vi.clearAllMocks();
  });

  // Mounts a host that calls useOverlayBackHandler, PROVIDING `reg` via
  // BACK_HANDLER_KEY (the production path is provide/inject, not a param).
  function mountWith(reg: BackHandlerRegistry, z: number) {
    return mount(
      defineComponent({
        setup() {
          useOverlayBackHandler(shown, onBack, z);
          return {};
        },
        template: "<div />",
      }),
      { global: { provide: { [BACK_HANDLER_KEY]: reg } } },
    );
  }

  it("does not push while hidden; pushes (onBack, z) when shown; pops when hidden again", async () => {
    const reg = makeFakeRegistry();
    mountWith(reg, Z.gate);
    await flushPromises();
    expect(reg.push).not.toHaveBeenCalled();

    shown.value = true;
    await flushPromises();
    expect(reg.push).toHaveBeenCalledWith(onBack, Z.gate);

    shown.value = false;
    await flushPromises();
    expect(reg.pop).toHaveBeenCalledWith(HANDLE);
  });

  it("pops on unmount while still shown (no leak)", async () => {
    const reg = makeFakeRegistry();
    const wrapper = mountWith(reg, Z.overlay);
    shown.value = true;
    await flushPromises();
    expect(reg.push).toHaveBeenCalledTimes(1);

    wrapper.unmount();
    await flushPromises();
    expect(reg.pop).toHaveBeenCalledWith(HANDLE);
  });
});
