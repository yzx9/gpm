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

vi.mock("@/i18n", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/i18n")>();
  return {
    ...actual,
    // Stub the cold-start reconcile so it doesn't fire an extra invoke that
    // would consume the test's sequenced invoke mocks.
    reconcileLocaleFromBackend: vi.fn().mockResolvedValue(undefined),
  };
});

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

    expect(invoke).toHaveBeenCalledWith("biometric_unlock", expect.anything());
  });

  it("stays in biometric mode when the prompt is cancelled (passphrase behind the switch)", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(true) // is_biometric_available
      .mockResolvedValueOnce(true) // is_biometric_unlock_enabled
      .mockRejectedValueOnce({
        code: "BIOMETRIC_CANCELLED",
        message: "cancel",
      }); // biometric_unlock (auto-prompt)
    const wrapper = mount(UnlockModal);
    await flushPromises();

    // No notice shown for a plain cancel.
    expect(wrapper.text()).not.toContain("Biometric was reset");
    // Cancel keeps the user in biometric mode: the primary is still present and
    // the passphrase input stays hidden behind the "Unlock with passphrase"
    // switch (tapping it reveals the form — covered by a later test).
    expect(wrapper.text()).toContain("Unlock with biometric");
    expect(wrapper.text()).toContain("Unlock with passphrase");
    expect(wrapper.find('input[type="password"]').exists()).toBe(false);
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
    // Auto-switched to passphrase mode (biometric is no longer viable): the
    // input is now visible and the "Unlock with biometric" switch is gone.
    expect(wrapper.find('input[type="password"]').exists()).toBe(true);
    expect(wrapper.text()).not.toContain("Unlock with biometric");
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

  it("wipes the typed passphrase on browser back (popstate)", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(false) // is_biometric_available
      .mockResolvedValueOnce(false); // is_biometric_unlock_enabled
    const wrapper = mount(UnlockModal);
    await flushPromises();

    const input = () =>
      wrapper.find('input[type="password"]').element as HTMLInputElement;
    await wrapper.find('input[type="password"]').setValue("topsecret");
    expect(input().value).toBe("topsecret");

    // vue-router is mocked, so drive popstate directly (the real browser-back path).
    window.dispatchEvent(new PopStateEvent("popstate"));
    await flushPromises();

    expect(input().value).toBe("");
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

    expect(invoke).toHaveBeenCalledWith("biometric_unlock", expect.anything());
  });

  it("reveals the passphrase form on tap and submits to unlock", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(true) // is_biometric_available
      .mockResolvedValueOnce(true) // is_biometric_unlock_enabled
      .mockRejectedValueOnce({ code: "BIOMETRIC_CANCELLED", message: "x" }) // auto-prompt
      .mockResolvedValueOnce({ lock_mode: "immediate" }); // get_app_config
    const wrapper = mount(UnlockModal);
    await flushPromises();

    // Starts in biometric mode (no passphrase input yet).
    expect(wrapper.find('input[type="password"]').exists()).toBe(false);

    // Tap the ghost switch to reveal the passphrase form.
    const switchBtn = wrapper
      .findAll("button")
      .find((b) => b.text().includes("Unlock with passphrase"))!;
    expect(switchBtn).toBeTruthy();
    await switchBtn.trigger("click");
    await flushPromises();

    // Input now present; submitting flows to unlock.
    expect(wrapper.find('input[type="password"]').exists()).toBe(true);
    await wrapper.find('input[type="password"]').setValue("secret");
    await wrapper.find("form").trigger("submit.prevent");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("unlock", { passphrase: "secret" });
  });

  it("offers a back-to-biometric action in passphrase mode that re-prompts", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(true) // is_biometric_available
      .mockResolvedValueOnce(true) // is_biometric_unlock_enabled
      .mockRejectedValueOnce({ code: "BIOMETRIC_CANCELLED", message: "x" }) // auto-prompt
      .mockResolvedValueOnce({ lock_mode: "immediate" }); // get_app_config
    const wrapper = mount(UnlockModal);
    await flushPromises();

    // Reveal the passphrase form first.
    await wrapper
      .findAll("button")
      .find((b) => b.text().includes("Unlock with passphrase"))!
      .trigger("click");
    await flushPromises();

    // The back-to-biometric switch is present in passphrase mode; tapping it
    // re-prompts biometric.
    const toBiometric = wrapper
      .findAll("button")
      .find((b) => b.text().includes("Unlock with biometric"))!;
    expect(toBiometric).toBeTruthy();
    await toBiometric.trigger("click");
    await flushPromises();

    const biometricCalls = vi
      .mocked(invoke)
      .mock.calls.filter((c) => c[0] === "biometric_unlock");
    expect(biometricCalls).toHaveLength(2); // auto-prompt + manual re-prompt
  });

  it("stays in biometric mode on a transient biometric error (lockout)", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(true) // is_biometric_available
      .mockResolvedValueOnce(true) // is_biometric_unlock_enabled
      .mockRejectedValueOnce({
        code: "BIOMETRIC_LOCKOUT",
        message: "Too many attempts, try later",
      }) // auto-prompt (transient)
      .mockResolvedValueOnce({ lock_mode: "immediate" }); // get_app_config
    const wrapper = mount(UnlockModal);
    await flushPromises();

    // Notice shown, but still in biometric mode — no auto-switch, no disable.
    expect(wrapper.text()).toContain("Too many attempts, try later");
    expect(wrapper.text()).toContain("Unlock with biometric");
    expect(wrapper.find('input[type="password"]').exists()).toBe(false);
    expect(invoke).not.toHaveBeenCalledWith("disable_biometric_unlock");
  });

  it("surfaces an error when the passphrase is wrong", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(false) // is_biometric_available
      .mockResolvedValueOnce(false) // is_biometric_unlock_enabled
      .mockResolvedValueOnce({ lock_mode: "immediate" }) // get_app_config
      .mockRejectedValueOnce({ code: "WRONG_PASSPHRASE", message: "nope" }); // unlock
    const wrapper = mount(UnlockModal);
    await flushPromises();

    await wrapper.find('input[type="password"]').setValue("wrong");
    await wrapper.find("form").trigger("submit.prevent");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("unlock", { passphrase: "wrong" });
    expect(wrapper.text()).toContain("Wrong passphrase");
  });

  it("the close (×) button emits `close` so the host can dismiss the overlay", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(false) // is_biometric_available
      .mockResolvedValueOnce(false) // is_biometric_unlock_enabled
      .mockResolvedValueOnce({ lock_mode: "immediate" }); // get_app_config
    const wrapper = mount(UnlockModal);
    await flushPromises();

    const closeBtn = wrapper.find('button[aria-label="Close"]');
    expect(closeBtn.exists()).toBe(true);
    await closeBtn.trigger("click");

    expect(wrapper.emitted("close")).toBeTruthy();
    expect(wrapper.emitted("close")).toHaveLength(1);
  });

  it("emits `close` on a backdrop tap (the BaseModalShell dismiss path)", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(false) // is_biometric_available
      .mockResolvedValueOnce(false) // is_biometric_unlock_enabled
      .mockResolvedValueOnce({ lock_mode: "immediate" }); // get_app_config
    const wrapper = mount(UnlockModal);
    await flushPromises();

    // BaseModalShell emits `close` on @click.self of its overlay (role=dialog).
    await wrapper.find('[role="dialog"]').trigger("click");

    expect(wrapper.emitted("close")).toBeTruthy();
    expect(wrapper.emitted("close")).toHaveLength(1);
  });

  it("shows the auto-lock policy hint for the Immediate default", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(false) // is_biometric_available
      .mockResolvedValueOnce(false) // is_biometric_unlock_enabled
      .mockResolvedValueOnce({ lock_mode: "immediate" }); // get_app_config
    const wrapper = mount(UnlockModal);
    await flushPromises();

    expect(wrapper.text()).toContain("Identity is cleared after every action.");
  });

  it("shows the idle-auto-lock hint for a timed policy", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(false) // is_biometric_available
      .mockResolvedValueOnce(false) // is_biometric_unlock_enabled
      .mockResolvedValueOnce({ lock_mode: { idle: 300 } }); // get_app_config
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
      .mockResolvedValueOnce({ lock_mode: "never" }); // get_app_config
    const wrapper = mount(UnlockModal);
    await flushPromises();

    expect(wrapper.text()).toContain(
      "Identity stays unlocked until you lock manually.",
    );
  });

  it("falls back to the Immediate hint when get_app_config fails", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(false) // is_biometric_available
      .mockResolvedValueOnce(false) // is_biometric_unlock_enabled
      .mockRejectedValueOnce(new Error("pre-setup")); // get_app_config rejects
    const wrapper = mount(UnlockModal);
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("get_app_config");
    expect(wrapper.text()).toContain("Identity is cleared after every action.");
  });

  it("the ? button toggles the passphrase explainer", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(false) // is_biometric_available
      .mockResolvedValueOnce(false) // is_biometric_unlock_enabled
      .mockResolvedValueOnce({ lock_mode: "immediate" }); // get_app_config
    const wrapper = mount(UnlockModal);
    await flushPromises();

    const helpBtn = wrapper.find(
      'button[aria-label="What is this passphrase?"]',
    );
    expect(helpBtn.exists()).toBe(true);
    expect(wrapper.text()).not.toContain("cannot recover or reset it");

    await helpBtn.trigger("click");
    expect(wrapper.text()).toContain("cannot recover or reset it");

    await helpBtn.trigger("click");
    expect(wrapper.text()).not.toContain("cannot recover or reset it");
  });

  it("does not expose a Reset affordance (recovery lives in Settings)", async () => {
    vi.mocked(invoke)
      .mockResolvedValueOnce(false) // is_biometric_available
      .mockResolvedValueOnce(false) // is_biometric_unlock_enabled
      .mockResolvedValueOnce({ lock_mode: "immediate" }); // get_app_config
    const wrapper = mount(UnlockModal);
    await flushPromises();

    const resetBtn = wrapper
      .findAll("button")
      .find((b) => b.text().includes("Reset all data"));
    expect(resetBtn).toBeUndefined();
    expect(invoke).not.toHaveBeenCalledWith("reset_config");
  });
});
