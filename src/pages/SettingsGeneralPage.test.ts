// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import BaseSegmentedControl from "@/components/base/BaseSegmentedControl.vue";
import { setLocale } from "@/i18n";
import { mountWithApp } from "@/test/appTestUtils";
import {
  baseDefaults,
  resetOverrides,
  type Overrides,
} from "@/test/settingsTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import SettingsGeneralPage from "./SettingsGeneralPage.vue";

const { mockPush, mockReplace } = vi.hoisted(() => ({
  mockPush: vi.fn(),
  mockReplace: vi.fn(),
}));

vi.mock("@tauri-apps/api/core");
// Stub @/i18n so the language-picker tests don't mutate the real i18n singleton.
vi.mock("@/i18n", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/i18n")>();
  return {
    ...actual,
    setLocale: vi.fn().mockResolvedValue(undefined),
    normalizeSupported: vi.fn((tag: string) => tag),
  };
});
vi.mock("vue-router", () => ({
  createRouter: vi.fn(),
  createWebHashHistory: vi.fn(),
  onBeforeRouteLeave: vi.fn(),
  useRouter: () => ({ push: mockPush, replace: mockReplace, back: vi.fn() }),
  useRoute: () => ({
    params: {},
    query: {},
    name: "",
    path: "/",
    fullPath: "/",
  }),
}));

describe("SettingsGeneralPage", () => {
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
    installMock();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  function mountPage() {
    return mountWithApp(SettingsGeneralPage).wrapper;
  }

  describe("reset", () => {
    async function openReset(wrapper: ReturnType<typeof mountPage>) {
      const dangerBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Reset All Data"));
      await dangerBtn!.trigger("click");
      await flushPromises();
    }

    function modalConfirmBtn(wrapper: ReturnType<typeof mountPage>) {
      return wrapper
        .find('[role="alertdialog"]')
        .findAll("button")
        .find((b) => b.text().includes("Reset"));
    }

    it("opens a type-RESET modal from the Danger Zone without wiping", async () => {
      const wrapper = mountPage();
      await flushPromises();
      expect(wrapper.find('[role="alertdialog"]').exists()).toBe(false);

      await openReset(wrapper);

      expect(wrapper.find('[role="alertdialog"]').exists()).toBe(true);
      expect(wrapper.text()).toContain("Type RESET to confirm");
      expect(invoke).not.toHaveBeenCalledWith("reset_config");
    });

    it("calls reset_config and navigates after typing RESET and confirming", async () => {
      when("reset_config", undefined);
      const wrapper = mountPage();
      await flushPromises();
      await openReset(wrapper);

      await wrapper.find('[role="alertdialog"] input').setValue("RESET");
      await modalConfirmBtn(wrapper)!.trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("reset_config");
      expect(mockReplace).toHaveBeenCalledWith({ name: "setup" });
    });

    it("keeps the confirm button disabled until RESET is typed", async () => {
      const wrapper = mountPage();
      await flushPromises();
      await openReset(wrapper);

      await wrapper.find('[role="alertdialog"] input').setValue("RESETT");
      expect(
        (modalConfirmBtn(wrapper)!.element as HTMLButtonElement).disabled,
      ).toBe(true);

      await wrapper.find('[role="alertdialog"] input').setValue("RESET");
      expect(
        (modalConfirmBtn(wrapper)!.element as HTMLButtonElement).disabled,
      ).toBe(false);

      const cancelBtn = wrapper
        .find('[role="alertdialog"]')
        .findAll("button")
        .find((b) => b.text().includes("Cancel"));
      await cancelBtn!.trigger("click");
      await flushPromises();

      expect(wrapper.find('[role="alertdialog"]').exists()).toBe(false);
      expect(invoke).not.toHaveBeenCalledWith("reset_config");
    });

    it("accepts case-insensitive, padded RESET", async () => {
      when("reset_config", undefined);
      const wrapper = mountPage();
      await flushPromises();
      await openReset(wrapper);

      await wrapper.find('[role="alertdialog"] input').setValue("  reset  ");
      expect(
        (modalConfirmBtn(wrapper)!.element as HTMLButtonElement).disabled,
      ).toBe(false);
      await modalConfirmBtn(wrapper)!.trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("reset_config");
    });

    it("shows error when reset fails", async () => {
      reject("reset_config", { code: "Err", message: "Reset failed" });
      const wrapper = mountPage();
      await flushPromises();
      await openReset(wrapper);

      await wrapper.find('[role="alertdialog"] input').setValue("RESET");
      await modalConfirmBtn(wrapper)!.trigger("click");
      await flushPromises();

      expect(wrapper.find("[role='alert']").text()).toContain("Reset failed");
      expect(wrapper.find('[role="alertdialog"]').exists()).toBe(false);
    });
  });

  describe("display-language picker", () => {
    function findLanguagePicker(wrapper: ReturnType<typeof mountPage>) {
      return (
        wrapper.findAllComponents(
          BaseSegmentedControl,
        ) as unknown as VueWrapper<any>[]
      ).find((c) => c.props("name") === "display-language");
    }

    it("applies a pinned locale in-memory first, then persists it", async () => {
      when("get_app_config", {}); // no locale ⇒ "system"
      const { wrapper, toast } = mountWithApp(SettingsGeneralPage);
      await flushPromises();

      const picker = findLanguagePicker(wrapper)!;
      picker.vm.$emit("change", "zh-CN");
      await flushPromises();

      expect(setLocale).toHaveBeenCalledWith("zh-CN");
      expect(invoke).toHaveBeenCalledWith("set_locale_pref", {
        locale: "zh-CN",
      });
      expect(
        toast.toasts.value.some((t) => t.message.includes("Display language")),
      ).toBe(true);
    });

    it("rolls back to the prior selection when persisting fails", async () => {
      when("get_app_config", { locale: "en" }); // prior = en
      reject("set_locale_pref", { code: "CONFIG_ERROR", message: "no" });
      const { wrapper, toast } = mountWithApp(SettingsGeneralPage);
      await flushPromises();

      const picker = findLanguagePicker(wrapper)!;
      picker.vm.$emit("change", "zh-CN");
      await flushPromises();

      expect(picker?.props("modelValue")).toBe("en");
      expect(
        toast.toasts.value.some((t) =>
          t.message.includes("Couldn't save display language"),
        ),
      ).toBe(true);
    });

    it("'system' resolves through the backend and clears the override", async () => {
      when("get_app_config", { locale: "en" });
      when("resolved_locale", "zh-CN");
      const wrapper = mountPage();
      await flushPromises();

      const picker = findLanguagePicker(wrapper)!;
      picker.vm.$emit("change", "system");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("resolved_locale");
      expect(setLocale).toHaveBeenCalledWith("zh-CN"); // normalizeSupported passthrough
      expect(invoke).toHaveBeenCalledWith("set_locale_pref", { locale: null });
    });
  });

  describe("theme picker", () => {
    // applyTheme mutates the real <html data-theme>; reset it between tests so
    // one test's pinned attribute can't leak into another's assertions.
    beforeEach(() => {
      delete document.documentElement.dataset.theme;
    });

    function findThemePicker(wrapper: ReturnType<typeof mountPage>) {
      return (
        wrapper.findAllComponents(
          BaseSegmentedControl,
        ) as unknown as VueWrapper<any>[]
      ).find((c) => c.props("name") === "theme-mode");
    }

    it("reflects the persisted theme_mode on load", async () => {
      when("get_app_config", { theme_mode: "dark" });
      const wrapper = mountPage();
      await flushPromises();

      expect(findThemePicker(wrapper)?.props("modelValue")).toBe("dark");
    });

    it("applies a pinned theme to <html data-theme> and persists it", async () => {
      when("get_app_config", {}); // no theme_mode ⇒ system
      const { wrapper, toast } = mountWithApp(SettingsGeneralPage);
      await flushPromises();

      const picker = findThemePicker(wrapper)!;
      picker.vm.$emit("change", "dark");
      await flushPromises();

      expect(document.documentElement.dataset.theme).toBe("dark");
      expect(invoke).toHaveBeenCalledWith("set_theme_mode", { mode: "dark" });
      expect(toast.toasts.value.some((t) => t.message.includes("Theme"))).toBe(
        true,
      );
    });

    it("rolls back the picker and the applied theme when persisting fails", async () => {
      when("get_app_config", { theme_mode: "light" }); // prior = light
      reject("set_theme_mode", { code: "CONFIG_ERROR", message: "no" });
      const { wrapper, toast } = mountWithApp(SettingsGeneralPage);
      await flushPromises();

      const picker = findThemePicker(wrapper)!;
      picker.vm.$emit("change", "dark"); // applies dark in-memory, then fails
      await flushPromises();

      expect(picker?.props("modelValue")).toBe("light");
      expect(document.documentElement.dataset.theme).toBe("light");
      expect(
        toast.toasts.value.some((t) =>
          t.message.includes("Couldn't save theme"),
        ),
      ).toBe(true);
    });

    it("'system' clears the override (persists null and removes the attribute)", async () => {
      when("get_app_config", { theme_mode: "dark" }); // prior = dark
      const wrapper = mountPage();
      await flushPromises();

      const picker = findThemePicker(wrapper)!;
      picker.vm.$emit("change", "system");
      await flushPromises();

      expect(document.documentElement.dataset.theme).toBeUndefined();
      expect(invoke).toHaveBeenCalledWith("set_theme_mode", { mode: null });
    });
  });

  describe("secure-screen picker", () => {
    function findSecurePicker(wrapper: ReturnType<typeof mountPage>) {
      return (
        wrapper.findAllComponents(
          BaseSegmentedControl,
        ) as unknown as VueWrapper<any>[]
      ).find((c) => c.props("name") === "secure-screen");
    }

    it("renders the three-state picker defaulting to sensitive", async () => {
      const wrapper = mountPage();
      await flushPromises();

      const picker = findSecurePicker(wrapper);
      expect(picker).toBeTruthy();
      expect(picker?.props("modelValue")).toBe("sensitive");
    });

    it("persists a new mode via set_secure_screen_mode and toasts", async () => {
      const { wrapper, toast } = mountWithApp(SettingsGeneralPage);
      await flushPromises();

      const picker = findSecurePicker(wrapper)!;
      picker.vm.$emit("change", "always");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("set_secure_screen_mode", {
        mode: "always",
      });
      expect(
        toast.toasts.value.some((t) => t.message.includes("every screen")),
      ).toBe(true);
    });
  });
});
