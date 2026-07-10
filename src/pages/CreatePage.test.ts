// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import type { CreatePreset } from "@/api";
import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises, type VueWrapper } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import CreatePage from "./CreatePage.vue";

vi.mock("@tauri-apps/api/core");

const { mockPush, mockReplace } = vi.hoisted(() => ({
  mockPush: vi.fn(),
  mockReplace: vi.fn(),
}));

vi.mock("vue-router", () => ({
  createRouter: vi.fn(),
  createWebHashHistory: vi.fn(),
  useRouter: () => ({ push: mockPush, replace: mockReplace, back: vi.fn() }),
  useRoute: () => ({
    params: {},
    query: {},
    name: "",
    path: "/",
    fullPath: "/",
  }),
}));

const preset = (over: Partial<CreatePreset> = {}): CreatePreset => ({
  id: "website-login",
  label: "Website Login",
  prefix: "websites",
  name_from: ["name"],
  fields: [],
  ...over,
});

// CreatePage is now the pick-only step (preset/custom/generate each route to
// their own page — see CreatePresetPage / CreateCustomPage / GeneratePasswordPage).
describe("CreatePage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "list_create_presets") return Promise.resolve([preset()]);
      return Promise.resolve(undefined);
    });
  });

  const card = (w: VueWrapper, text: string) =>
    w.findAll("button").find((b) => b.text().includes(text));

  it("loads presets and renders them as pick cards", async () => {
    const w = mountWithApp(CreatePage).wrapper;
    await flushPromises();
    expect(w.text()).toContain("Website Login");
    expect(w.text()).toContain("Custom secret");
    expect(w.text()).toContain("Generate password");
  });

  it("tapping a preset routes to the preset form", async () => {
    const w = mountWithApp(CreatePage).wrapper;
    await flushPromises();
    await card(w, "Website Login")!.trigger("click");
    expect(mockPush).toHaveBeenCalledWith({
      name: "createPreset",
      params: { presetId: "website-login" },
    });
  });

  it("tapping Custom routes to the custom form", async () => {
    const w = mountWithApp(CreatePage).wrapper;
    await flushPromises();
    await card(w, "Custom secret")!.trigger("click");
    expect(mockPush).toHaveBeenCalledWith({ name: "createCustom" });
  });

  it("tapping Generate routes to the standalone generator", async () => {
    const w = mountWithApp(CreatePage).wrapper;
    await flushPromises();
    await card(w, "Generate password")!.trigger("click");
    expect(mockPush).toHaveBeenCalledWith({ name: "generate" });
  });

  it("Back pops to entries (navBack falls to replace at the deep-link root)", async () => {
    const w = mountWithApp(CreatePage).wrapper;
    await flushPromises();
    await w.find('button[aria-label="Back"]').trigger("click");
    await flushPromises();
    expect(mockReplace).toHaveBeenCalledWith({ name: "entries" });
  });
});
