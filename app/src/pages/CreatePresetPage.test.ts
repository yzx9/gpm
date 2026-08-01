// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import type { CreatePreset } from "@/api";
import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import CreatePresetPage from "./CreatePresetPage.vue";

vi.mock("@tauri-apps/api/core");

const { mockReplace, route } = vi.hoisted(() => ({
  mockReplace: vi.fn(),
  route: {
    params: { presetId: "website-login" },
    query: {},
    name: "createPreset",
    path: "/",
    fullPath: "/",
  },
}));

vi.mock("vue-router", () => ({
  createRouter: vi.fn(),
  createWebHashHistory: vi.fn(),
  useRouter: () => ({ push: vi.fn(), replace: mockReplace, back: vi.fn() }),
  useRoute: () => route,
}));

const preset = (): CreatePreset => ({
  id: "website-login",
  label: "Website Login",
  prefix: "websites",
  name_from: ["name"],
  fields: [
    {
      key: "name",
      label: "Name",
      required: true,
      type: "string",
      charset: null,
      min: null,
      max: null,
      strict: false,
    },
  ],
});

describe("CreatePresetPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    route.params.presetId = "website-login";
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "list_create_presets") return Promise.resolve([preset()]);
      if (cmd === "create_from_preset_secret")
        return Promise.resolve({ kind: "written", commit: "abc1234" });
      return Promise.resolve(undefined);
    });
  });

  it("loads the preset and renders its required field", async () => {
    const w = mountWithApp(CreatePresetPage).wrapper;
    await flushPromises();
    expect(w.text()).toContain("Name");
  });

  it("redirects to /create when the preset id is unknown", async () => {
    route.params.presetId = "bogus";
    mountWithApp(CreatePresetPage);
    await flushPromises();
    expect(mockReplace).toHaveBeenCalledWith({ name: "create" });
  });

  it("Save creates the secret and returns to entries", async () => {
    const w = mountWithApp(CreatePresetPage).wrapper;
    await flushPromises();
    await w.find('input[id="f-name"]').setValue("github");
    await w.find("form").trigger("submit");
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith("create_from_preset_secret", {
      presetId: "website-login",
      fields: { name: "github" },
    });
    expect(mockReplace).toHaveBeenCalledWith({ name: "entries" });
  });

  it("Back returns to the pick step", async () => {
    const w = mountWithApp(CreatePresetPage).wrapper;
    await flushPromises();
    await w.find('button[aria-label="Back"]').trigger("click");
    await flushPromises();
    expect(mockReplace).toHaveBeenCalledWith({ name: "create" });
  });
});
