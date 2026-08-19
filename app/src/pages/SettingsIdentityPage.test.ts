// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import BaseSegmentedControl from "@/components/base/BaseSegmentedControl.vue";
import BaseSelect from "@/components/base/BaseSelect.vue";
import { mountWithApp } from "@/test/appTestUtils";
import {
  baseDefaults,
  resetOverrides,
  type Overrides,
} from "@/test/settingsTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import SettingsIdentityPage from "./SettingsIdentityPage.vue";

const { mockPush, mockReplace, mockOnBeforeRouteLeave, mockRoute } = vi.hoisted(
  () => ({
    mockPush: vi.fn(),
    mockReplace: vi.fn(),
    mockOnBeforeRouteLeave: vi.fn(),
    // Mutable route so a test can set `mockRoute.query = { focus: ... }` before
    // mounting to exercise the deep-link scroll/highlight.
    mockRoute: {
      params: {},
      query: {} as Record<string, unknown>,
      name: "",
      path: "/",
      fullPath: "/",
    },
  }),
);

vi.mock("@tauri-apps/api/core");
vi.mock("vue-router", () => ({
  createRouter: vi.fn(),
  createWebHashHistory: vi.fn(),
  onBeforeRouteLeave: mockOnBeforeRouteLeave,
  useRouter: () => ({ push: mockPush, replace: mockReplace, back: vi.fn() }),
  useRoute: () => mockRoute,
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

  // Find a BaseSegmentedControl / BaseSelect by its `name` prop.
  function findControl(
    wrapper: ReturnType<typeof mountPage>,
    Comp: typeof BaseSelect | typeof BaseSegmentedControl,
    name: string,
  ) {
    return (
      wrapper.findAllComponents(Comp) as unknown as VueWrapper<any>[]
    ).find((c) => c.props("name") === name);
  }

  describe("deep-link focus (?focus=...)", () => {
    it("focus=biometric scrolls to the biometric card and flashes it", async () => {
      // scrollIntoView is undefined in jsdom — stub it on Element so the call is
      // observable, then restore it in finally.
      const proto = Element.prototype as { scrollIntoView?: unknown };
      const orig = proto.scrollIntoView;
      const scrollIntoView = vi.fn();
      proto.scrollIntoView = scrollIntoView;
      mockRoute.query = { focus: "biometric" };
      mockRoute.name = "settingsIdentity"; // applyFocus only clears the query on this route
      when("get_auth_state", {
        configured: true,
        encrypted: true,
        unlocked: false,
        identity_type: "x25519",
      });
      const { wrapper, stackedRouterView } = mountWithApp(SettingsIdentityPage);
      try {
        await flushPromises(); // settle onMounted: loadConfig → applyFocus polls, then parks on the slide-settle wait
        // Pinned: the scroll/highlight must NOT fire until the slide settles —
        // dropping the whenSettled await (the fix) would call it eagerly here.
        expect(scrollIntoView).not.toHaveBeenCalled();
        stackedRouterView.releaseEnter(); // resolve whenSettled (page tests don't mount the <Transition>)
        await flushPromises(); // query clear + scroll + highlight apply
        const card = wrapper.find("#biometric-card");
        expect(card.exists()).toBe(true);
        expect(scrollIntoView).toHaveBeenCalledWith({
          behavior: "smooth",
          block: "center",
        });
        expect(card.classes()).toContain("card-highlight");
        expect(mockReplace).toHaveBeenCalledWith({ query: {} });
        vi.advanceTimersByTime(1700); // the flash auto-clears
        await flushPromises();
        expect(wrapper.find("#biometric-card").classes()).not.toContain(
          "card-highlight",
        );
      } finally {
        wrapper.unmount(); // cancels the highlight-clear timer
        proto.scrollIntoView = orig;
        mockRoute.query = {};
        mockRoute.name = "";
      }
    });

    it("focus=passphrase scrolls to the passphrase card and flashes it", async () => {
      const proto = Element.prototype as { scrollIntoView?: unknown };
      const orig = proto.scrollIntoView;
      const scrollIntoView = vi.fn();
      proto.scrollIntoView = scrollIntoView;
      mockRoute.query = { focus: "passphrase" };
      mockRoute.name = "settingsIdentity";
      when("get_auth_state", {
        configured: true,
        encrypted: false, // unencrypted x25519 → the Set Passphrase card renders
        unlocked: false,
        identity_type: "x25519",
      });
      const { wrapper, stackedRouterView } = mountWithApp(SettingsIdentityPage);
      try {
        await flushPromises();
        expect(scrollIntoView).not.toHaveBeenCalled();
        stackedRouterView.releaseEnter();
        await flushPromises();
        const card = wrapper.find("#passphrase-card");
        expect(card.exists()).toBe(true);
        expect(scrollIntoView).toHaveBeenCalledWith({
          behavior: "smooth",
          block: "center",
        });
        expect(card.classes()).toContain("card-highlight");
        expect(mockReplace).toHaveBeenCalledWith({ query: {} });
      } finally {
        wrapper.unmount();
        proto.scrollIntoView = orig;
        mockRoute.query = {};
        mockRoute.name = "";
      }
    });

    it("does not scroll or highlight if the route moved off before the slide settled", async () => {
      // F3 pin: a back-nav during the slide cancels the enter, which resolves
      // the whenSettled awaiter BEFORE onUnmounted runs — so `alive` is still
      // true when applyFocus resumes, but currentRoute already points elsewhere.
      // The scroll/highlight must bail on the route, not just on `alive`.
      const proto = Element.prototype as { scrollIntoView?: unknown };
      const orig = proto.scrollIntoView;
      const scrollIntoView = vi.fn();
      proto.scrollIntoView = scrollIntoView;
      mockRoute.query = { focus: "biometric" };
      mockRoute.name = "settingsIdentity";
      when("get_auth_state", {
        configured: true,
        encrypted: true,
        unlocked: false,
        identity_type: "x25519",
      });
      const { wrapper, stackedRouterView } = mountWithApp(SettingsIdentityPage);
      try {
        await flushPromises(); // onMounted → loadConfig → applyFocus parks on whenSettled
        expect(scrollIntoView).not.toHaveBeenCalled();
        // The back-nav has already moved the route off this page (synchronously)
        // by the time the cancelled-enter awaiter is released.
        mockRoute.name = "permissions";
        stackedRouterView.releaseEnter();
        await flushPromises();
        expect(scrollIntoView).not.toHaveBeenCalled();
        expect(wrapper.find("#biometric-card").classes()).not.toContain(
          "card-highlight",
        );
      } finally {
        wrapper.unmount();
        proto.scrollIntoView = orig;
        mockRoute.query = {};
        mockRoute.name = "";
      }
    });
  });

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
    it("renders the auto-lock 3-way primary defaulting to Immediate (idle select hidden)", async () => {
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).toContain("Auto-Lock & Auto-Clear");
      const lock = findControl(wrapper, BaseSegmentedControl, "lock-mode");
      expect(lock?.props("modelValue")).toBe("immediate");
      expect(lock?.props("options")).toHaveLength(3);
      // The idle-duration select is hidden unless the mode is "After idle".
      expect(findControl(wrapper, BaseSelect, "lock-idle")).toBeUndefined();
    });

    it("switching auto-lock to After idle persists the restored idle duration", async () => {
      when("set_lock_mode", { lock_mode: { idle: 60 } });
      const wrapper = mountPage();
      await flushPromises();

      // "After idle" → restores the default 1 min idle.
      await findControl(wrapper, BaseSegmentedControl, "lock-mode")!.vm.$emit(
        "change",
        "idle",
      );
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("set_lock_mode", {
        mode: { idle: 60 },
      });
    });

    it("the idle-duration select persists a new idle and restores it on re-entry", async () => {
      // Start in "After idle" at 1 min.
      when("get_app_config", { lock_mode: { idle: 60 } });
      when("set_lock_mode", { lock_mode: { idle: 900 } });
      const wrapper = mountPage();
      await flushPromises();

      const idle = findControl(wrapper, BaseSelect, "lock-idle")!;
      expect(idle).toBeTruthy(); // shown because the mode is idle
      await idle.vm.$emit("change", { idle: 900 }); // 15 min
      await flushPromises();
      expect(invoke).toHaveBeenCalledWith("set_lock_mode", {
        mode: { idle: 900 },
      });

      // Round-trip Immediate → After idle restores the 15 min, not the default.
      vi.mocked(invoke).mockClear();
      when("set_lock_mode", { lock_mode: "immediate" });
      await findControl(wrapper, BaseSegmentedControl, "lock-mode")!.vm.$emit(
        "change",
        "immediate",
      );
      await flushPromises();
      when("set_lock_mode", { lock_mode: { idle: 900 } });
      await findControl(wrapper, BaseSegmentedControl, "lock-mode")!.vm.$emit(
        "change",
        "idle",
      );
      await flushPromises();
      expect(invoke).toHaveBeenCalledWith("set_lock_mode", {
        mode: { idle: 900 },
      });
    });

    it("view-clear on/off toggles and the duration select persists", async () => {
      when("get_app_config", { view_clear_secs: null }); // 45s default → on
      when("set_view_clear_secs", { view_clear_secs: 10 });
      const wrapper = mountPage();
      await flushPromises();

      const toggle = findControl(wrapper, BaseSegmentedControl, "view-clear")!;
      expect(toggle.props("modelValue")).toBe(true);
      expect(
        findControl(wrapper, BaseSelect, "view-clear-duration"),
      ).toBeTruthy();

      await findControl(wrapper, BaseSelect, "view-clear-duration")!.vm.$emit(
        "change",
        10,
      );
      await flushPromises();
      expect(invoke).toHaveBeenCalledWith("set_view_clear_secs", { secs: 10 });

      await toggle.vm.$emit("change", false); // off → 0
      await flushPromises();
      expect(invoke).toHaveBeenCalledWith("set_view_clear_secs", { secs: 0 });
    });

    it("clipboard-clear on/off toggles and the duration select persists", async () => {
      when("get_app_config", { clipboard_clear_secs: null });
      when("set_clipboard_clear_secs", { clipboard_clear_secs: 180 });
      const wrapper = mountPage();
      await flushPromises();

      const toggle = findControl(
        wrapper,
        BaseSegmentedControl,
        "clipboard-clear",
      )!;
      expect(toggle.props("modelValue")).toBe(true);

      await findControl(
        wrapper,
        BaseSelect,
        "clipboard-clear-duration",
      )!.vm.$emit("change", 180);
      await flushPromises();
      expect(invoke).toHaveBeenCalledWith("set_clipboard_clear_secs", {
        secs: 180,
      });

      await toggle.vm.$emit("change", false);
      await flushPromises();
      expect(invoke).toHaveBeenCalledWith("set_clipboard_clear_secs", {
        secs: 0,
      });
    });

    it("switching auto-lock to Never persists 'never' and hides the idle select", async () => {
      when("get_app_config", { lock_mode: { idle: 60 } }); // start in After idle
      when("set_lock_mode", { lock_mode: "never" });
      const wrapper = mountPage();
      await flushPromises();

      expect(findControl(wrapper, BaseSelect, "lock-idle")).toBeTruthy();
      await findControl(wrapper, BaseSegmentedControl, "lock-mode")!.vm.$emit(
        "change",
        "never",
      );
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("set_lock_mode", { mode: "never" });
      expect(findControl(wrapper, BaseSelect, "lock-idle")).toBeUndefined();
    });

    it("switching to Never confirms before persisting", async () => {
      when("get_app_config", { lock_mode: { idle: 60 } });
      when("set_lock_mode", { lock_mode: "never" });
      const { wrapper, dialog } = mountWithApp(SettingsIdentityPage);
      await flushPromises();

      await findControl(wrapper, BaseSegmentedControl, "lock-mode")!.vm.$emit(
        "change",
        "never",
      );
      await flushPromises();

      expect(dialog.dialog.confirm).toHaveBeenCalledWith(
        expect.objectContaining({ danger: true }),
      );
      expect(invoke).toHaveBeenCalledWith("set_lock_mode", { mode: "never" });
    });

    it("canceling the Never confirm keeps the current mode", async () => {
      when("get_app_config", { lock_mode: { idle: 60 } });
      const { wrapper, dialog } = mountWithApp(SettingsIdentityPage);
      vi.mocked(dialog.dialog.confirm).mockResolvedValue(false);
      await flushPromises();

      await findControl(wrapper, BaseSegmentedControl, "lock-mode")!.vm.$emit(
        "change",
        "never",
      );
      await flushPromises();

      expect(dialog.dialog.confirm).toHaveBeenCalled();
      expect(invoke).not.toHaveBeenCalledWith(
        "set_lock_mode",
        expect.objectContaining({ mode: "never" }),
      );
      // Controlled pill stays on the prior (idle) mode.
      expect(
        findControl(wrapper, BaseSegmentedControl, "lock-mode")!.props(
          "modelValue",
        ),
      ).toBe("idle");
    });

    it("switching to a non-Never mode does not prompt", async () => {
      when("get_app_config", { lock_mode: { idle: 60 } });
      when("set_lock_mode", { lock_mode: "immediate" });
      const { wrapper, dialog } = mountWithApp(SettingsIdentityPage);
      await flushPromises();

      await findControl(wrapper, BaseSegmentedControl, "lock-mode")!.vm.$emit(
        "change",
        "immediate",
      );
      await flushPromises();

      expect(dialog.dialog.confirm).not.toHaveBeenCalled();
      expect(invoke).toHaveBeenCalledWith("set_lock_mode", {
        mode: "immediate",
      });
    });

    it("turning clipboard-clear off confirms before persisting", async () => {
      when("get_app_config", { clipboard_clear_secs: null });
      when("set_clipboard_clear_secs", { clipboard_clear_secs: 0 });
      const { wrapper, dialog } = mountWithApp(SettingsIdentityPage);
      await flushPromises();

      await findControl(
        wrapper,
        BaseSegmentedControl,
        "clipboard-clear",
      )!.vm.$emit("change", false);
      await flushPromises();

      expect(dialog.dialog.confirm).toHaveBeenCalledWith(
        expect.objectContaining({ danger: true }),
      );
      expect(invoke).toHaveBeenCalledWith("set_clipboard_clear_secs", {
        secs: 0,
      });
    });

    it("canceling the clipboard-clear off confirm keeps it on", async () => {
      when("get_app_config", { clipboard_clear_secs: null });
      const { wrapper, dialog } = mountWithApp(SettingsIdentityPage);
      vi.mocked(dialog.dialog.confirm).mockResolvedValue(false);
      await flushPromises();

      await findControl(
        wrapper,
        BaseSegmentedControl,
        "clipboard-clear",
      )!.vm.$emit("change", false);
      await flushPromises();

      expect(dialog.dialog.confirm).toHaveBeenCalled();
      expect(invoke).not.toHaveBeenCalledWith(
        "set_clipboard_clear_secs",
        expect.objectContaining({ secs: 0 }),
      );
      expect(
        findControl(wrapper, BaseSegmentedControl, "clipboard-clear")!.props(
          "modelValue",
        ),
      ).toBe(true);
    });

    it("view-clear off→on restores the last-used duration, not the default", async () => {
      when("get_app_config", { view_clear_secs: null });
      when("set_view_clear_secs", { view_clear_secs: 180 });
      const wrapper = mountPage();
      await flushPromises();

      // Pick a non-default duration (3 min).
      await findControl(wrapper, BaseSelect, "view-clear-duration")!.vm.$emit(
        "change",
        180,
      );
      await flushPromises();
      // Toggle off, then back on → must restore 180, not the 45s default.
      await findControl(wrapper, BaseSegmentedControl, "view-clear")!.vm.$emit(
        "change",
        false,
      );
      await flushPromises();
      vi.mocked(invoke).mockClear();
      await findControl(wrapper, BaseSegmentedControl, "view-clear")!.vm.$emit(
        "change",
        true,
      );
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("set_view_clear_secs", {
        secs: 180,
      });
    });

    it("clipboard-clear off→on restores the last-used duration, not the default", async () => {
      when("get_app_config", { clipboard_clear_secs: null });
      when("set_clipboard_clear_secs", { clipboard_clear_secs: 180 });
      const wrapper = mountPage();
      await flushPromises();

      await findControl(
        wrapper,
        BaseSelect,
        "clipboard-clear-duration",
      )!.vm.$emit("change", 180);
      await flushPromises();
      await findControl(
        wrapper,
        BaseSegmentedControl,
        "clipboard-clear",
      )!.vm.$emit("change", false);
      await flushPromises();
      vi.mocked(invoke).mockClear();
      await findControl(
        wrapper,
        BaseSegmentedControl,
        "clipboard-clear",
      )!.vm.$emit("change", true);
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("set_clipboard_clear_secs", {
        secs: 180,
      });
    });

    it("surfaces an error when a view-clear duration pick fails to persist", async () => {
      when("get_app_config", { view_clear_secs: null });
      reject("set_view_clear_secs", { code: "CONFIG_ERROR", message: "nope" });
      const wrapper = mountPage();
      await flushPromises();

      await findControl(wrapper, BaseSelect, "view-clear-duration")!.vm.$emit(
        "change",
        10,
      );
      await flushPromises();

      expect(wrapper.find("[role='alert']").text()).toContain("nope");
    });
  });

  describe("app lock: re-lock when inactive & coupling", () => {
    it("renders the gate-idle on/off + duration and invokes set_gate_idle", async () => {
      when("is_app_lock_available", true);
      when("get_app_lock_state", { enabled: true, locked: false });
      when("set_gate_idle", { gate_idle: { after: 900 } });
      const wrapper = mountPage();
      await flushPromises();

      // App Lock on → gate-idle shows on/off primary; default {after:300} → on.
      const toggle = findControl(wrapper, BaseSegmentedControl, "gate-idle");
      expect(toggle?.props("modelValue")).toBe(true);
      const after = findControl(wrapper, BaseSelect, "gate-idle-after");
      expect(after).toBeTruthy();

      await after!.vm.$emit("change", { after: 900 }); // 15 min
      await flushPromises();
      expect(invoke).toHaveBeenCalledWith("set_gate_idle", {
        mode: { after: 900 },
      });
    });

    it("gate-idle toggle off persists the off sentinel and hides the after select", async () => {
      when("is_app_lock_available", true);
      when("get_app_lock_state", { enabled: true, locked: false });
      when("set_gate_idle", { gate_idle: "off" });
      const wrapper = mountPage();
      await flushPromises();

      expect(findControl(wrapper, BaseSelect, "gate-idle-after")).toBeTruthy();
      await findControl(wrapper, BaseSegmentedControl, "gate-idle")!.vm.$emit(
        "change",
        false,
      );
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("set_gate_idle", { mode: "off" });
      expect(
        findControl(wrapper, BaseSelect, "gate-idle-after"),
      ).toBeUndefined();
    });

    it("gate-idle off→on restores the last-used after duration, not the default", async () => {
      when("is_app_lock_available", true);
      when("get_app_lock_state", { enabled: true, locked: false });
      when("set_gate_idle", { gate_idle: { after: 1800 } });
      const wrapper = mountPage();
      await flushPromises();

      // Pick 30 min.
      await findControl(wrapper, BaseSelect, "gate-idle-after")!.vm.$emit(
        "change",
        { after: 1800 },
      );
      await flushPromises();
      // Toggle off, then on → restore 1800, not the 5 min default.
      await findControl(wrapper, BaseSegmentedControl, "gate-idle")!.vm.$emit(
        "change",
        false,
      );
      await flushPromises();
      vi.mocked(invoke).mockClear();
      await findControl(wrapper, BaseSegmentedControl, "gate-idle")!.vm.$emit(
        "change",
        true,
      );
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("set_gate_idle", {
        mode: { after: 1800 },
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
      expect(wrapper.findAll('input[name="gate-idle"]')).toHaveLength(2);
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

  // ── Gate re-lock (issue #20): the mask does not unmount the page ────────

  it("a gate re-lock closes the passphrase modal and wipes the typed values, without marking the drafts notice", async () => {
    // F3B: the closed modal IS the explanation — wipeSecrets returns void, so
    // no post-unlock toast for this page (deliberate; see the issue #20
    // review).
    const m = mountWithApp(SettingsIdentityPage);
    await flushPromises();

    const setBtn = m.wrapper
      .findAll("button")
      .find((b) => b.text().includes("Set Passphrase"));
    await setBtn!.trigger("click");
    await flushPromises();
    await m.wrapper.find("#pp-new").setValue("secret");
    expect(m.wrapper.find('[role="dialog"]').exists()).toBe(true);

    m.appLock.setAppLocked(true, "idle");
    await flushPromises();

    // Fields wiped + modal closed by wipeSecrets itself.
    expect(m.wrapper.find('[role="dialog"]').exists()).toBe(false);
    // …but no draft-loss notice (F3B).
    expect(m.draftsNotice.consume()).toBe(false);
  });
});
