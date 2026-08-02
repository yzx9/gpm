// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { mountWithApp } from "@/test/appTestUtils";
import {
  baseDefaults,
  resetOverrides,
  type Overrides,
} from "@/test/settingsTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import SettingsIdentityPage from "./SettingsIdentityPage.vue";

const { mockPush, mockReplace, mockOnBeforeRouteLeave } = vi.hoisted(() => ({
  mockPush: vi.fn(),
  mockReplace: vi.fn(),
  mockOnBeforeRouteLeave: vi.fn(),
}));

vi.mock("@tauri-apps/api/core");
vi.mock("vue-router", () => ({
  createRouter: vi.fn(),
  createWebHashHistory: vi.fn(),
  onBeforeRouteLeave: mockOnBeforeRouteLeave,
  useRouter: () => ({ push: mockPush, replace: mockReplace, back: vi.fn() }),
  useRoute: () => ({
    params: {},
    query: {},
    name: "",
    path: "/",
    fullPath: "/",
  }),
}));

describe("SettingsIdentityPage", () => {
  const overrides: Overrides = {};
  const defaults = { ...baseDefaults };

  function when(cmd: string, value: unknown) {
    overrides[cmd] = { value };
  }
  function reject(cmd: string, payload: unknown) {
    overrides[cmd] = { reject: payload };
  }
  function installMock() {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd in overrides) {
        const o = overrides[cmd];
        if (o && o.reject !== undefined) return Promise.reject(o.reject);
        return Promise.resolve(o ? o.value : defaults[cmd]);
      }
      return Promise.resolve(defaults[cmd]);
    });
  }

  beforeEach(() => {
    vi.clearAllMocks();
    resetOverrides(overrides);
    vi.useFakeTimers();
    vi.stubGlobal(
      "navigator",
      Object.assign(navigator, {
        clipboard: { writeText: vi.fn().mockResolvedValue(undefined) },
      }),
    );
    installMock();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  function mountPage() {
    return mountWithApp(SettingsIdentityPage).wrapper;
  }

  describe("identity passphrase", () => {
    it("set passphrase: blocks Encrypt until the unrecoverable ack is checked", async () => {
      const wrapper = mountPage();
      await flushPromises();
      const openBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Set Passphrase"))!;
      await openBtn.trigger("click");
      await flushPromises();
      const modal = wrapper.find('[role="dialog"]');
      const modalBtn = (text: string) =>
        modal.findAll("button").find((b) => b.text().includes(text))!;

      await modal.find('input[id="pp-new"]').setValue("secret");
      await modal.find('input[id="pp-new-confirm"]').setValue("secret");

      const ack = modal.find('input[type="checkbox"]');
      expect(ack.exists()).toBe(true);
      expect((ack.element as HTMLInputElement).checked).toBe(false);
      expect(
        (modalBtn("Encrypt Identity").element as HTMLButtonElement).disabled,
      ).toBe(true);
      await modalBtn("Encrypt Identity").trigger("click");
      await flushPromises();
      expect(invoke).not.toHaveBeenCalledWith(
        "set_passphrase",
        expect.anything(),
      );

      await ack.setValue(true);
      when("set_passphrase", { ok: true });
      await modalBtn("Encrypt Identity").trigger("click");
      await flushPromises();
      expect(invoke).toHaveBeenCalledWith("set_passphrase", {
        passphrase: "secret",
      });
    });

    it("set passphrase: editing the passphrase after acking forces a re-ack", async () => {
      const wrapper = mountPage();
      await flushPromises();
      const openBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Set Passphrase"))!;
      await openBtn.trigger("click");
      await flushPromises();
      const modal = wrapper.find('[role="dialog"]');
      const modalBtn = (text: string) =>
        modal.findAll("button").find((b) => b.text().includes(text))!;
      await modal.find('input[id="pp-new"]').setValue("secret");
      await modal.find('input[id="pp-new-confirm"]').setValue("secret");
      await modal.find('input[type="checkbox"]').setValue(true);
      expect(
        (modalBtn("Encrypt Identity").element as HTMLButtonElement).disabled,
      ).toBe(false);

      await modal.find('input[id="pp-new"]').setValue("changed");
      await modal.find('input[id="pp-new-confirm"]').setValue("changed");
      expect(
        (modal.find('input[type="checkbox"]').element as HTMLInputElement)
          .checked,
      ).toBe(false);
      expect(
        (modalBtn("Encrypt Identity").element as HTMLButtonElement).disabled,
      ).toBe(true);
    });

    it("set passphrase: blocks encrypt when the confirm does not match", async () => {
      const wrapper = mountPage();
      await flushPromises();
      const openBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Set Passphrase"))!;
      await openBtn.trigger("click");
      await flushPromises();
      const modal = wrapper.find('[role="dialog"]');
      const modalBtn = (text: string) =>
        modal.findAll("button").find((b) => b.text().includes(text))!;

      await modal.find('input[id="pp-new"]').setValue("secret");
      await modal.find('input[id="pp-new-confirm"]').setValue("different");
      await modal.find('input[type="checkbox"]').setValue(true);
      await modalBtn("Encrypt Identity").trigger("click");
      await flushPromises();

      expect(invoke).not.toHaveBeenCalledWith(
        "set_passphrase",
        expect.anything(),
      );
      expect(wrapper.text()).toContain("Passphrases do not match");
    });

    it("change passphrase: submit is gated on the unrecoverable ack too", async () => {
      when("get_auth_state", {
        configured: true,
        encrypted: true,
        unlocked: true,
        identity_type: "x25519",
      });
      const wrapper = mountPage();
      await flushPromises();
      const openBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Change Passphrase"))!;
      await openBtn.trigger("click");
      await flushPromises();
      const modal = wrapper.find('[role="dialog"]');
      const modalBtn = (text: string) =>
        modal.findAll("button").find((b) => b.text().includes(text))!;

      await modal.find('input[id="pp-current"]').setValue("old-pass");
      await modal.find('input[id="pp-new"]').setValue("new-pass");
      await modal.find('input[id="pp-new-confirm"]').setValue("new-pass");

      const ack = modal.find('input[type="checkbox"]');
      expect(
        (modalBtn("Change Passphrase").element as HTMLButtonElement).disabled,
      ).toBe(true);
      await ack.setValue(true);
      when("change_passphrase", { ok: true });
      await modalBtn("Change Passphrase").trigger("click");
      await flushPromises();
      expect(invoke).toHaveBeenCalledWith("change_passphrase", {
        oldPassphrase: "old-pass",
        newPassphrase: "new-pass",
      });
    });

    it("enable-biometric modal does not show the unrecoverable ack", async () => {
      when("get_auth_state", {
        configured: true,
        encrypted: true,
        unlocked: true,
        identity_type: "x25519",
      });
      when("is_biometric_available", "available");
      const wrapper = mountPage();
      await flushPromises();
      const openBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Enable Biometric"))!;
      await openBtn.trigger("click");
      await flushPromises();

      const modal = wrapper.find('[role="dialog"]');
      expect(modal.text()).not.toContain("cannot be recovered");
      expect(modal.find('input[type="checkbox"]').exists()).toBe(false);
    });
  });

  describe("passphrase modal", () => {
    it("cancel wipes the typed passphrase", async () => {
      const wrapper = mountPage();
      await flushPromises();

      const setBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Set Passphrase"));
      await setBtn!.trigger("click");
      await flushPromises();

      const modal = wrapper.find('[role="dialog"]');
      expect(modal.exists()).toBe(true);
      await modal.find("#pp-new").setValue("secret");
      await modal.find('input[type="checkbox"]').setValue(true);
      await modal
        .findAll("button")
        .find((b) => b.text().includes("Cancel"))!
        .trigger("click");
      await flushPromises();

      expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
      expect(invoke).not.toHaveBeenCalledWith(
        "set_passphrase",
        expect.anything(),
      );

      await setBtn!.trigger("click");
      await flushPromises();
      expect((wrapper.find("#pp-new").element as HTMLInputElement).value).toBe(
        "",
      );
      expect(
        (
          wrapper.find('[role="dialog"]').find('input[type="checkbox"]')
            .element as HTMLInputElement
        ).checked,
      ).toBe(false);
    });

    it("backdrop dismisses without invoking", async () => {
      const wrapper = mountPage();
      await flushPromises();

      const setBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Set Passphrase"));
      await setBtn!.trigger("click");
      await flushPromises();

      await wrapper.find('[role="dialog"]').trigger("click");
      await flushPromises();

      expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
      expect(invoke).not.toHaveBeenCalledWith(
        "set_passphrase",
        expect.anything(),
      );
    });
  });

  describe("biometric unlock card", () => {
    const encryptedAuth = {
      configured: true,
      encrypted: true,
      unlocked: false,
      identity_type: "x25519",
    };

    it("is hidden when the identity is not encrypted", async () => {
      when("is_biometric_available", "available");
      when("is_biometric_unlock_enabled", true);
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).not.toContain("Biometric Unlock");
    });

    it("reports unavailable when no biometric is present", async () => {
      when("get_auth_state", encryptedAuth);
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).toContain("Biometric Unlock");
      expect(wrapper.text()).toContain("isn't available on this device");
    });

    it("calls enable_biometric_unlock with the passphrase when enabling", async () => {
      when("get_auth_state", encryptedAuth);
      when("is_biometric_available", "available");
      when("is_biometric_unlock_enabled", false);
      when("enable_biometric_unlock", undefined);
      const { wrapper, toast } = mountWithApp(SettingsIdentityPage);
      await flushPromises();

      const enableBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Enable Biometric"));
      await enableBtn!.trigger("click");
      await flushPromises();

      const modal = wrapper.find('[role="dialog"]');
      expect(modal.exists()).toBe(true);
      await modal.find("#pp-current").setValue("my-pass");
      await modal
        .findAll("button")
        .find((b) => b.text().includes("Enable Biometric"))!
        .trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith(
        "enable_biometric_unlock",
        expect.objectContaining({ passphrase: "my-pass" }),
      );
      expect(
        toast.toasts.value.some((t) =>
          t.message.includes("Biometric unlock enabled"),
        ),
      ).toBe(true);
    });

    it("shows an error on a wrong passphrase when enabling", async () => {
      when("get_auth_state", encryptedAuth);
      when("is_biometric_available", "available");
      when("is_biometric_unlock_enabled", false);
      reject("enable_biometric_unlock", {
        code: "WRONG_PASSPHRASE",
        message: "wrong",
      });
      const wrapper = mountPage();
      await flushPromises();

      const enableBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Enable Biometric"));
      await enableBtn!.trigger("click");
      await flushPromises();

      const modal = wrapper.find('[role="dialog"]');
      await modal.find("#pp-current").setValue("bad");
      await modal
        .findAll("button")
        .find((b) => b.text().includes("Enable Biometric"))!
        .trigger("click");
      await flushPromises();

      expect(wrapper.find('[role="dialog"]').exists()).toBe(true);
      expect(wrapper.find("[role='alert']").text()).toContain(
        "Wrong passphrase",
      );
    });

    it("calls disable_biometric_unlock after confirming when disabling", async () => {
      when("get_auth_state", encryptedAuth);
      when("is_biometric_available", "available");
      when("is_biometric_unlock_enabled", true);
      when("disable_biometric_unlock", undefined);
      const { wrapper, dialog } = mountWithApp(SettingsIdentityPage);
      await flushPromises();

      const disableBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Disable Biometric"));
      expect(disableBtn).toBeDefined();
      await disableBtn!.trigger("click");
      await flushPromises();

      // The disable is now gated behind a destructive confirm.
      expect(dialog.dialog.confirm).toHaveBeenCalledWith(
        expect.objectContaining({ danger: true }),
      );
      expect(invoke).toHaveBeenCalledWith("disable_biometric_unlock");
    });

    it("does not disable when the confirm is cancelled", async () => {
      when("get_auth_state", encryptedAuth);
      when("is_biometric_available", "available");
      when("is_biometric_unlock_enabled", true);
      const { wrapper, dialog } = mountWithApp(SettingsIdentityPage);
      // mountWithApp defaults confirm to "proceed"; flip it to cancel.
      vi.mocked(dialog.dialog.confirm).mockResolvedValue(false);
      await flushPromises();

      const disableBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Disable Biometric"));
      await disableBtn!.trigger("click");
      await flushPromises();

      expect(dialog.dialog.confirm).toHaveBeenCalled();
      expect(invoke).not.toHaveBeenCalledWith("disable_biometric_unlock");
    });
  });

  describe("identity auto-unlock disable", () => {
    it("calls disable_identity_auto_unlock after confirming", async () => {
      when("is_app_lock_available", true);
      when("get_app_lock_state", { enabled: true, locked: false });
      when("get_auth_state", {
        configured: true,
        encrypted: true,
        unlocked: false,
        identity_type: "x25519",
      });
      when("get_config", { unlock_identity_with_app: true });
      when("disable_identity_auto_unlock", undefined);
      const { wrapper, dialog } = mountWithApp(SettingsIdentityPage);
      await flushPromises();

      const disableBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Disable Auto-Unlock"));
      await disableBtn!.trigger("click");
      await flushPromises();

      expect(dialog.dialog.confirm).toHaveBeenCalledWith(
        expect.objectContaining({ danger: true }),
      );
      expect(invoke).toHaveBeenCalledWith("disable_identity_auto_unlock");
    });
  });

  describe("auto-lock & auto-clear card", () => {
    it("renders the three controls", async () => {
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).toContain("Auto-Lock & Auto-Clear");
      expect(wrapper.findAll('input[name="lock-mode"]')).toHaveLength(6);
      expect(wrapper.findAll('input[name="view-clear"]')).toHaveLength(4);
      expect(wrapper.findAll('input[name="clipboard-clear"]')).toHaveLength(3);
    });

    it("switching the auto-lock mode invokes set_lock_mode", async () => {
      when("set_lock_mode", { lock_mode: { idle: 60 } });
      const wrapper = mountPage();
      await flushPromises();

      // radios[1] is the "1 min" preset ({ idle: 60 }).
      await wrapper.findAll('input[name="lock-mode"]')[1]!.trigger("change");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("set_lock_mode", {
        mode: { idle: 60 },
      });
    });

    it("switching the view auto-clear invokes set_view_clear_secs", async () => {
      when("set_view_clear_secs", { view_clear_secs: 10 });
      const wrapper = mountPage();
      await flushPromises();

      // radios[0] is the "10s" preset (value 10).
      await wrapper.findAll('input[name="view-clear"]')[0]!.trigger("change");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("set_view_clear_secs", { secs: 10 });
    });
  });

  describe("app lock: re-lock when inactive & coupling", () => {
    it("renders the gate-idle control and invokes set_gate_idle", async () => {
      when("is_app_lock_available", true);
      when("get_app_lock_state", { enabled: true, locked: false });
      when("set_gate_idle", { gate_idle: { after: 900 } });
      const wrapper = mountPage();
      await flushPromises();

      // App Lock on → the "Re-lock when inactive" control shows 4 presets.
      expect(wrapper.findAll('input[name="gate-idle"]')).toHaveLength(4);
      // radios[2] is the "15 min" preset ({ after: 900 }).
      await wrapper.findAll('input[name="gate-idle"]')[2]!.trigger("change");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("set_gate_idle", {
        mode: { after: 900 },
      });
    });

    it("disables the identity auto-lock while the identity is coupled to App Lock", async () => {
      when("is_app_lock_available", true);
      when("get_app_lock_state", { enabled: true, locked: false });
      when("get_auth_state", {
        configured: true,
        encrypted: true,
        unlocked: false,
        identity_type: "x25519",
      });
      when("get_config", { unlock_identity_with_app: true });
      const wrapper = mountPage();
      await flushPromises();

      // identityCoupled (gate on + auto-unlock on + encrypted) → the managed
      // note shows and the lock-mode fieldset is disabled (the radios rely on
      // the fieldset's disabled state, so check the fieldset, not each radio).
      expect(wrapper.text()).toContain(
        "Ignored while Identity Auto-Unlock is on",
      );
      const lockModeField = wrapper
        .find('input[name="lock-mode"]')
        .element.closest("fieldset") as HTMLFieldSetElement | null;
      expect(lockModeField?.disabled).toBe(true);
    });

    it("keeps the identity auto-lock enabled when the gate is off (not coupled)", async () => {
      when("get_auth_state", {
        configured: true,
        encrypted: true,
        unlocked: false,
        identity_type: "x25519",
      });
      when("get_config", { unlock_identity_with_app: true });
      // gate stays off (default get_app_lock_state.enabled = false) → not coupled
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).not.toContain(
        "Ignored while Identity Auto-Unlock is on",
      );
      const fs = wrapper
        .find('input[name="lock-mode"]')
        .element.closest("fieldset") as HTMLFieldSetElement | null;
      expect(fs?.disabled).toBe(false);
    });

    it("hides the auto-unlock section when the gate is on but the identity is not encrypted", async () => {
      when("is_app_lock_available", true);
      when("get_app_lock_state", { enabled: true, locked: false });
      // default get_auth_state → encrypted: false
      const wrapper = mountPage();
      await flushPromises();

      // gate-idle control still shows; the auto-unlock opt-in does not
      expect(wrapper.findAll('input[name="gate-idle"]')).toHaveLength(4);
      expect(
        wrapper.findAll("button").some((b) => b.text().includes("Auto-Unlock")),
      ).toBe(false);
    });
  });

  describe("passphrase change re-fetches auto-unlock state", () => {
    it("calls get_config after change_passphrase (re-encryption can revoke the sealed slot)", async () => {
      when("get_auth_state", {
        configured: true,
        encrypted: true,
        unlocked: true,
        identity_type: "x25519",
      });
      when("change_passphrase", { ok: true });
      when("get_config", { unlock_identity_with_app: false });
      const wrapper = mountPage();
      await flushPromises();
      // Clear loadConfig's calls so only the submit's invokes are asserted.
      vi.mocked(invoke).mockClear();

      const openBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Change Passphrase"))!;
      await openBtn.trigger("click");
      await flushPromises();
      const modal = wrapper.find('[role="dialog"]');
      await modal.find('input[id="pp-current"]').setValue("old");
      await modal.find('input[id="pp-new"]').setValue("newpass");
      await modal.find('input[id="pp-new-confirm"]').setValue("newpass");
      await modal.find('input[type="checkbox"]').setValue(true);
      await modal
        .findAll("button")
        .find((b) => b.text().includes("Change Passphrase"))!
        .trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("get_config");
    });
  });
});
