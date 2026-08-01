// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import AddKeyPage from "./AddKeyPage.vue";

vi.mock("@tauri-apps/api/core");

const { mockReplace } = vi.hoisted(() => ({ mockReplace: vi.fn() }));

vi.mock("vue-router", () => ({
  createRouter: vi.fn(),
  createWebHashHistory: vi.fn(),
  useRouter: () => ({ push: vi.fn(), replace: mockReplace, back: vi.fn() }),
  useRoute: () => ({
    params: {},
    query: {},
    name: "",
    path: "/",
    fullPath: "/",
  }),
}));

describe("AddKeyPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(invoke).mockResolvedValue(undefined);
  });

  it("Save is disabled until a key is pasted", async () => {
    const w = mountWithApp(AddKeyPage).wrapper;
    await flushPromises();
    const save = w
      .findAll("button")
      .find((b) => b.text().includes("Save key"))!;
    expect((save.element as HTMLButtonElement).disabled).toBe(true);
    await w.find("textarea").setValue("ssh-ed25519 AAAA key");
    expect((save.element as HTMLButtonElement).disabled).toBe(false);
  });

  it("Save adds the key and returns to settings", async () => {
    const w = mountWithApp(AddKeyPage).wrapper;
    await flushPromises();
    await w.find("textarea").setValue("ssh-ed25519 AAAA key");
    await w.find('input[type="text"]').setValue("Alice — laptop");
    await w.find("form").trigger("submit");
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith("add_trusted_signing_key", {
      armored: "ssh-ed25519 AAAA key",
      label: "Alice — laptop",
    });
    expect(mockReplace).toHaveBeenCalledWith({ name: "settings" });
  });
});
