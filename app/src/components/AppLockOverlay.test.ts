// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { mountWithApp } from "@/test/appTestUtils";
import { Z } from "@/zTiers";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import AppLockOverlay from "./AppLockOverlay.vue";

vi.mock("@tauri-apps/api/core");
// Stub the cold-start locale reconcile so it doesn't fire an extra invoke that
// would consume the test's sequenced invoke mocks (same pattern as UnlockModal).
vi.mock("@/i18n", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/i18n")>();
  return {
    ...actual,
    reconcileLocaleFromBackend: vi.fn().mockResolvedValue(undefined),
  };
});
// The prompt-text builder is only passed into the (mocked) app_unlock invoke;
// stub it so its construction can't reach into native/i18n bridges under test.
vi.mock("@/i18n/native", () => ({
  appLockUnlockPrompt: vi.fn(() => ({
    title: "t",
    subtitle: "s",
    negative: "n",
  })),
}));

describe("AppLockOverlay", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // Default: app_unlock + export_diagnostics resolve. Tests override per case.
    vi.mocked(invoke).mockResolvedValue(undefined);
  });

  it("renders the fullscreen opaque gate with the diagnostics link (no card frame)", async () => {
    const { wrapper } = mountWithApp(AppLockOverlay);
    await flushPromises();

    expect(wrapper.find(".overlay.fullscreen").exists()).toBe(true);
    // fullscreen renders the slot directly — no BaseCard (.card) wrapper.
    expect(wrapper.find(".card").exists()).toBe(false);
    expect(wrapper.text()).toContain("gpm");
    expect(wrapper.text()).toContain("Export diagnostics");
  });

  it("the diagnostics link exports with the confirm stacked at Z.gate", async () => {
    const { wrapper, dialog } = mountWithApp(AppLockOverlay);
    await flushPromises();

    const link = wrapper
      .findAll("button")
      .find((b) => b.text().includes("Export diagnostics"))!;
    expect(link).toBeTruthy();
    await link.trigger("click");
    await flushPromises();

    // z: Z.gate so the confirm (and its success toast) surface above this gate.
    expect(dialog.dialog.confirm).toHaveBeenCalledWith(
      expect.objectContaining({ z: Z.gate }),
    );
    expect(invoke).toHaveBeenCalledWith("export_diagnostics");
  });

  it("KEYSTORE_UNAVAILABLE shows the dedicated (non-dead-end) notice", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "app_unlock") {
        return Promise.reject({
          code: "KEYSTORE_UNAVAILABLE",
          message: "no sensor",
        });
      }
      return Promise.resolve(undefined);
    });
    const { wrapper } = mountWithApp(AppLockOverlay);
    await flushPromises();

    // Tap Unlock so the notice shows whether or not the cold-start auto-prompt
    // already fired on mount.
    const unlock = wrapper
      .findAll("button")
      .find((b) => b.text().includes("Unlock with biometric"))!;
    await unlock.trigger("click");
    await flushPromises();

    expect(wrapper.text()).toContain(
      "Biometric unlock isn't available right now.",
    );
  });
});
