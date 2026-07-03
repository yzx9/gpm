// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import UnlockModal from "./UnlockModal.vue";

const { mockPush } = vi.hoisted(() => ({
  mockPush: vi.fn(),
}));

vi.mock("@tauri-apps/api/core");
vi.mock("vue-router", () => ({
  createRouter: vi.fn(),
  createWebHashHistory: vi.fn(),
  useRouter: () => ({
    push: mockPush,
    replace: vi.fn(),
    back: vi.fn(),
  }),
  useRoute: () => ({
    params: {},
    query: {},
    name: "",
    path: "/",
    fullPath: "/",
  }),
}));

// The modal only issues unlock commands; whether the overlay then hides is the
// backend's call (it emits `identity-lock-state`), driven by `App.vue`'s `v-if`
// and unit-tested in `useLockState.test.ts`. So these cases assert the command
// the modal fires, not global lock state.
describe("UnlockModal", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("auto-prompts biometric when enabled + available", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(true) // is_biometric_available
      .mockResolvedValueOnce(true) // is_biometric_unlock_enabled
      .mockResolvedValueOnce(undefined); // biometric_unlock (auto-prompt)
    mount(UnlockModal);
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("biometric_unlock");
  });

  it("keeps the passphrase form silently when biometric is cancelled", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(true) // is_biometric_available
      .mockResolvedValueOnce(true) // is_biometric_unlock_enabled
      .mockRejectedValueOnce({
        code: "BIOMETRIC_CANCELLED",
        message: "cancel",
      }); // biometric_unlock
    const wrapper = mount(UnlockModal);
    await flushPromises();

    // No notice shown for a plain cancel.
    expect(wrapper.text()).not.toContain("Biometric was reset");
    // The passphrase form is always present.
    expect(wrapper.find('input[type="password"]').exists()).toBe(true);
  });

  it("shows a reset notice and disables biometric when the key was invalidated", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(true) // is_biometric_available
      .mockResolvedValueOnce(true) // is_biometric_unlock_enabled
      .mockRejectedValueOnce({
        code: "BIOMETRIC_KEY_INVALIDATED",
        message: "invalidated",
      }) // biometric_unlock
      .mockResolvedValueOnce(undefined); // disable_biometric_unlock (self-heal)
    const wrapper = mount(UnlockModal);
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("disable_biometric_unlock");
    expect(wrapper.text()).toContain("Biometric was reset");
  });

  it("does not auto-prompt when biometric is unavailable", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(false) // is_biometric_available
      .mockResolvedValueOnce(false); // is_biometric_unlock_enabled
    const wrapper = mount(UnlockModal);
    await flushPromises();

    expect(invoke).not.toHaveBeenCalledWith("biometric_unlock");
    // No biometric button shown when not available/enabled.
    expect(wrapper.text()).not.toContain("Unlock with biometric");
    expect(wrapper.find('input[type="password"]').exists()).toBe(true);
  });

  it("submits the passphrase to the unlock command", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(false) // is_biometric_available
      .mockResolvedValueOnce(false) // is_biometric_unlock_enabled
      .mockResolvedValueOnce(undefined); // unlock
    const wrapper = mount(UnlockModal);
    await flushPromises();

    await wrapper.find('input[type="password"]').setValue("secret");
    await wrapper.find("form").trigger("submit.prevent");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("unlock", { passphrase: "secret" });
  });

  it("triggers biometric unlock from the biometric button", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(true) // is_biometric_available
      .mockResolvedValueOnce(true) // is_biometric_unlock_enabled
      .mockRejectedValueOnce({ code: "BIOMETRIC_CANCELLED", message: "x" }) // auto-prompt
      .mockResolvedValueOnce(undefined); // manual button -> biometric_unlock
    const wrapper = mount(UnlockModal);
    await flushPromises();

    const btn = wrapper
      .findAll("button")
      .find((b) => b.text().includes("Unlock with biometric"))!;
    expect(btn).toBeTruthy();
    await btn.trigger("click");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("biometric_unlock");
  });

  it("the close (×) button emits `close` so the host can dismiss the overlay", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(false) // is_biometric_available
      .mockResolvedValueOnce(false); // is_biometric_unlock_enabled
    const wrapper = mount(UnlockModal);
    await flushPromises();

    const closeBtn = wrapper.find('button[aria-label="Close"]');
    expect(closeBtn.exists()).toBe(true);
    await closeBtn.trigger("click");

    expect(wrapper.emitted("close")).toBeTruthy();
    expect(wrapper.emitted("close")).toHaveLength(1);
  });

  it("shows the auto-lock policy hint for the Immediate default", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(false) // is_biometric_available
      .mockResolvedValueOnce(false) // is_biometric_unlock_enabled
      .mockResolvedValueOnce({ lock_mode: "immediate" }); // get_config
    const wrapper = mount(UnlockModal);
    await flushPromises();

    expect(wrapper.text()).toContain(
      "Identity is cleared after every action.",
    );
  });

  it("shows the idle-auto-lock hint for a timed policy", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(false) // is_biometric_available
      .mockResolvedValueOnce(false) // is_biometric_unlock_enabled
      .mockResolvedValueOnce({ lock_mode: { idle: 300 } }); // get_config
    const wrapper = mount(UnlockModal);
    await flushPromises();

    expect(wrapper.text()).toContain(
      "Identity auto-locks after 5 min of inactivity.",
    );
  });

  it("shows the never-lock hint for the Never policy", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(false) // is_biometric_available
      .mockResolvedValueOnce(false) // is_biometric_unlock_enabled
      .mockResolvedValueOnce({ lock_mode: "never" }); // get_config
    const wrapper = mount(UnlockModal);
    await flushPromises();

    expect(wrapper.text()).toContain(
      "Identity stays unlocked until you lock manually.",
    );
  });

  it("does not expose a Reset affordance (recovery lives in Settings)", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(false) // is_biometric_available
      .mockResolvedValueOnce(false); // is_biometric_unlock_enabled
    const wrapper = mount(UnlockModal);
    await flushPromises();

    const resetBtn = wrapper
      .findAll("button")
      .find((b) => b.text().includes("Reset all data"));
    expect(resetBtn).toBeUndefined();
    expect(invoke).not.toHaveBeenCalledWith("reset_config");
  });
});
