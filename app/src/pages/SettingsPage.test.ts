// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { mountWithApp } from "@/test/appTestUtils";
import {
  baseDefaults,
  resetOverrides,
  when,
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

  it("renders the hub rows", async () => {
    const wrapper = mountPage();
    await flushPromises();

    expect(wrapper.findAll(".hub-row")).toHaveLength(7);
  });

  it("navigates into a category on row click", async () => {
    const wrapper = mountPage();
    await flushPromises();

    await wrapper.findAll(".hub-row")[0]!.trigger("click");
    expect(mockPush).toHaveBeenCalledWith({ name: "settingsGeneral" });

    // The 2nd row is Lock & Identity (the merged page).
    await wrapper.findAll(".hub-row")[1]!.trigger("click");
    expect(mockPush).toHaveBeenCalledWith({ name: "settingsIdentity" });

    await wrapper.findAll(".hub-row")[2]!.trigger("click");
    expect(mockPush).toHaveBeenCalledWith({ name: "settingsRepository" });

    // The 4th row is the diagnostics log viewer — leads the docs group
    // (Logs/Security/Permissions/About) below the settings categories.
    await wrapper.findAll(".hub-row")[3]!.trigger("click");
    expect(mockPush).toHaveBeenCalledWith({ name: "log" });

    // The 5th row is Security (plain-language explainer; no secret content).
    await wrapper.findAll(".hub-row")[4]!.trigger("click");
    expect(mockPush).toHaveBeenCalledWith({ name: "security" });

    // The 6th row is Permissions & data.
    await wrapper.findAll(".hub-row")[5]!.trigger("click");
    expect(mockPush).toHaveBeenCalledWith({ name: "settingsPermissions" });

    // The 7th row is About (overview/licenses; no secret content).
    await wrapper.findAll(".hub-row")[6]!.trigger("click");
    expect(mockPush).toHaveBeenCalledWith({ name: "about" });
  });

  it("navigates back to entries when Back is clicked", async () => {
    const wrapper = mountPage();
    await flushPromises();

    await wrapper.find('button[aria-label="Back"]').trigger("click");

    // navBack falls back to replace when there is no history to pop.
    expect(mockReplace).toHaveBeenCalledWith({ name: "entries" });
  });

  // RFC R090: a red dot on the About row signals an unacknowledged newer
  // release. Decorative — the About page carries the labeled Update action.
  it("shows an update dot on the About row when a release is unacknowledged", async () => {
    when(overrides, "get_update_status", {
      available: true,
      unacknowledged: true,
      latest_version: "v0.19.0",
    });
    const wrapper = mountPage();
    await flushPromises();

    const aboutRow = wrapper.findAll(".hub-row")[6]!;
    expect(aboutRow.find(".update-dot").exists()).toBe(true);
  });

  it("omits the About update dot when the release is already acknowledged", async () => {
    when(overrides, "get_update_status", {
      available: true,
      unacknowledged: false,
      latest_version: "v0.19.0",
    });
    const wrapper = mountPage();
    await flushPromises();

    expect(wrapper.findAll(".hub-row")[6]!.find(".update-dot").exists()).toBe(
      false,
    );
  });
});
