// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import CreateCustomPage from "./CreateCustomPage.vue";

vi.mock("@tauri-apps/api/core");

const { mockReplace } = vi.hoisted(() => ({ mockReplace: vi.fn() }));

vi.mock("vue-router", () => ({
  createRouter: vi.fn(),
  createWebHashHistory: vi.fn(),
  useRouter: () => ({ push: vi.fn(), replace: mockReplace, back: vi.fn() }),
  useRoute: () => ({
    params: {},
    query: {},
    name: "createCustom",
    path: "/",
    fullPath: "/",
  }),
}));

describe("CreateCustomPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "create_secret")
        return Promise.resolve({ kind: "written", commit: "abc1234" });
      if (cmd === "lookup_template") return Promise.resolve(null);
      if (cmd === "preview_create") return Promise.resolve(null);
      return Promise.resolve(undefined);
    });
  });

  it("Save is disabled until both name and content are filled", async () => {
    const w = mountWithApp(CreateCustomPage).wrapper;
    await flushPromises();
    const save = w
      .findAll("button")
      .find((b) => b.text().includes("Save secret"))!;
    expect((save.element as HTMLButtonElement).disabled).toBe(true);
    await w.find('input[id="c-name"]').setValue("misc/foo");
    expect((save.element as HTMLButtonElement).disabled).toBe(true);
    await w.find('textarea[id="c-content"]').setValue("hunter2");
    expect((save.element as HTMLButtonElement).disabled).toBe(false);
  });

  it("Save creates the secret and returns to entries", async () => {
    const w = mountWithApp(CreateCustomPage).wrapper;
    await flushPromises();
    await w.find('input[id="c-name"]').setValue("misc/foo");
    await w.find('textarea[id="c-content"]').setValue("hunter2");
    await w.find("form").trigger("submit");
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith("create_secret", {
      name: "misc/foo",
      content: "hunter2",
    });
    expect(mockReplace).toHaveBeenCalledWith({ name: "entries" });
  });

  it("Back returns to the pick step", async () => {
    const w = mountWithApp(CreateCustomPage).wrapper;
    await flushPromises();
    await w.find('button[aria-label="Back"]').trigger("click");
    await flushPromises();
    expect(mockReplace).toHaveBeenCalledWith({ name: "create" });
  });
});
