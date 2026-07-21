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
import SettingsLockingPage from "./SettingsLockingPage.vue";

const { mockPush, mockReplace } = vi.hoisted(() => ({
  mockPush: vi.fn(),
  mockReplace: vi.fn(),
}));

vi.mock("@tauri-apps/api/core");
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

describe("SettingsLockingPage", () => {
  const overrides: Overrides = {};
  const defaults = { ...baseDefaults };

  function when(cmd: string, value: unknown) {
    overrides[cmd] = { value };
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
    return mountWithApp(SettingsLockingPage).wrapper;
  }

  it("renders the auto-lock card with its three controls", async () => {
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
