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
import SettingsPage from "./SettingsPage.vue";

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

describe("SettingsPage (hub)", () => {
  const overrides: Overrides = {};
  const defaults = { ...baseDefaults };

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
    return mountWithApp(SettingsPage).wrapper;
  }

  it("renders the six hub rows", async () => {
    const wrapper = mountPage();
    await flushPromises();

    expect(wrapper.findAll(".hub-row")).toHaveLength(6);
    // The hub loads the summary sources.
    expect(invoke).toHaveBeenCalledWith("get_app_config");
    expect(invoke).toHaveBeenCalledWith("get_config");
    expect(invoke).toHaveBeenCalledWith("get_auth_state");
  });

  it("navigates into a category on row click", async () => {
    const wrapper = mountPage();
    await flushPromises();

    await wrapper.findAll(".hub-row")[0]!.trigger("click");
    expect(mockPush).toHaveBeenCalledWith({ name: "settingsGeneral" });

    await wrapper.findAll(".hub-row")[3]!.trigger("click");
    expect(mockPush).toHaveBeenCalledWith({ name: "settingsRepository" });

    // The 5th row is About (overview/licenses; no secret content).
    await wrapper.findAll(".hub-row")[4]!.trigger("click");
    expect(mockPush).toHaveBeenCalledWith({ name: "about" });

    // The 6th row is the diagnostics log viewer.
    await wrapper.findAll(".hub-row")[5]!.trigger("click");
    expect(mockPush).toHaveBeenCalledWith({ name: "log" });
  });

  it("navigates back to entries when Back is clicked", async () => {
    const wrapper = mountPage();
    await flushPromises();

    await wrapper.find('button[aria-label="Back"]').trigger("click");

    // navBack falls back to replace when there is no history to pop.
    expect(mockReplace).toHaveBeenCalledWith({ name: "entries" });
  });

  it("shows a repo-host summary on the Repository row", async () => {
    const wrapper = mountPage();
    await flushPromises();

    // httpsConfig.url = https://github.com/user/repo.git → github.com/user/repo
    expect(wrapper.findAll(".hub-row")[3]!.text()).toContain(
      "github.com/user/repo",
    );
  });
});
