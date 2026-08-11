// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

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

const passwordPreset = (): CreatePreset => ({
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
    {
      key: "password",
      label: "Password",
      required: true,
      type: "password",
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

  it("the per-field style picker changes the generator mode", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "list_create_presets")
        return Promise.resolve([passwordPreset()]);
      if (cmd === "generate_password") return Promise.resolve("generated-pw");
      return Promise.resolve(undefined);
    });
    const w = mountWithApp(CreatePresetPage).wrapper;
    await flushPromises();

    // A password field (charset == null) renders the BaseSelect style picker.
    expect(w.find('button[aria-label="Password style"]').exists()).toBe(true);

    // Open the sheet and pick xkcd (Passphrase, the 3rd option).
    await w.find('button[aria-label="Password style"]').trigger("click");
    await flushPromises();
    await w
      .findAll('input[type="radio"][name="gen-mode-password"]')[2]!
      .trigger("change");
    await flushPromises();

    // Generate for the password field — the picked mode reaches the backend.
    await w.find('button[aria-label="Generate password"]').trigger("click");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith(
      "generate_password",
      expect.objectContaining({ mode: "xkcd" }),
    );
  });

  it("renders a warning when the store needs an age plugin binary", async () => {
    // create_from_preset_secret rejects PLUGIN_UNAVAILABLE: the store has an age
    // plugin recipient whose binary can't run here. The alert must be a warning
    // (role=status) carrying the backend message, not a red danger (role=alert).
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "list_create_presets") return Promise.resolve([preset()]);
      if (cmd === "create_from_preset_secret")
        return Promise.reject({
          code: "PLUGIN_UNAVAILABLE",
          message:
            "Encryption needs the age plugin 'age-plugin-yubikey', which can't run on Android",
        });
      return Promise.resolve(undefined);
    });
    const w = mountWithApp(CreatePresetPage).wrapper;
    await flushPromises();
    await w.find('input[id="f-name"]').setValue("github");
    await w.find("form").trigger("submit");
    await flushPromises();

    const alert = w.find("[role='status']");
    expect(alert.exists()).toBe(true);
    expect(alert.text()).toContain("age-plugin-yubikey");
    expect(w.find("[role='alert']").exists()).toBe(false);
  });

  it("renders a red error for a generic create failure (baseline)", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "list_create_presets") return Promise.resolve([preset()]);
      if (cmd === "create_from_preset_secret")
        return Promise.reject({ code: "DECRYPT_FAILED", message: "boom" });
      return Promise.resolve(undefined);
    });
    const w = mountWithApp(CreatePresetPage).wrapper;
    await flushPromises();
    await w.find('input[id="f-name"]').setValue("github");
    await w.find("form").trigger("submit");
    await flushPromises();

    expect(w.find("[role='alert']").exists()).toBe(true);
    expect(w.find("[role='status']").exists()).toBe(false);
  });
});
