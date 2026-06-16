// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount, flushPromises, type DOMWrapper } from "@vue/test-utils";
import { invoke } from "@tauri-apps/api/core";
import CreatePage from "./CreatePage.vue";

const { mockPush } = vi.hoisted(() => ({ mockPush: vi.fn() }));

vi.mock("@tauri-apps/api/core");

vi.mock("vue-router", () => ({
  createRouter: vi.fn(),
  createWebHashHistory: vi.fn(),
  useRouter: () => ({ push: mockPush, replace: vi.fn(), back: vi.fn() }),
  useRoute: () => ({
    params: {},
    query: {},
    name: "create",
    path: "/create",
    fullPath: "/create",
  }),
}));

const WEBSITE_PRESET = {
  id: "website",
  label: "Website login",
  prefix: "websites",
  name_from: ["url", "username"],
  fields: [
    { key: "url", label: "Website URL", required: true },
    { key: "username", label: "Username", required: true },
    { key: "password", label: "Password", required: true },
  ],
} as const;

/** Find the first button whose text contains `needle`. */
function findButton(wrapper: ReturnType<typeof mount>, needle: string) {
  return wrapper.findAll("button").find((b) => b.text().includes(needle)) as
    | DOMWrapper<HTMLButtonElement>
    | undefined;
}

/** Wire `invoke` per command; returns a fresh mounted CreatePage. */
async function mountPage(handlers: Record<string, () => unknown>) {
  vi.mocked(invoke).mockImplementation(((cmd: string) => {
    const h = handlers[cmd];
    if (h) return Promise.resolve(h());
    if (cmd === "list_create_presets") return Promise.resolve([WEBSITE_PRESET]);
    return Promise.resolve(undefined);
  }) as typeof invoke);
  const wrapper = mount(CreatePage);
  await flushPromises(); // loadPresets on mount
  return wrapper;
}

async function fillWebsiteForm(wrapper: ReturnType<typeof mount>) {
  await wrapper.findAll(".type-card")[0]!.trigger("click");
  await wrapper.find("#f-url").setValue("example.com");
  await wrapper.find("#f-username").setValue("alice");
  await wrapper.find("#f-password").setValue("hunter2");
}

describe("CreatePage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("loads presets on mount", async () => {
    await mountPage({});
    expect(invoke).toHaveBeenCalledWith("list_create_presets");
  });

  it("creates a secret from a preset and navigates to the list", async () => {
    const wrapper = await mountPage({
      create_from_preset_secret: () => ({ kind: "written", commit: "abc1234" }),
    });

    await fillWebsiteForm(wrapper);
    await wrapper.find("form").trigger("submit");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("create_from_preset_secret", {
      presetId: "website",
      fields: { url: "example.com", username: "alice", password: "hunter2" },
    });
    expect(mockPush).toHaveBeenCalledWith({ name: "entries" });
  });

  it("creates a custom secret via create_secret", async () => {
    const wrapper = await mountPage({
      create_secret: () => ({ kind: "written", commit: "c1" }),
    });

    await wrapper.findAll(".type-card")[1]!.trigger("click"); // Custom secret
    await wrapper.find("#c-name").setValue("servers/db1");
    await wrapper.find("#c-content").setValue("master-password");
    await wrapper.find("form").trigger("submit");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("create_secret", {
      name: "servers/db1",
      content: "master-password",
    });
    expect(mockPush).toHaveBeenCalledWith({ name: "entries" });
  });

  it("on a decryptable conflict, Keep mine resolves and navigates", async () => {
    const wrapper = await mountPage({
      create_from_preset_secret: () => ({
        kind: "conflict",
        name: "websites/example.com/alice",
        remote_decryptable: true,
      }),
      resolve_write_conflict: () => ({ commit: "def5678" }),
    });

    await fillWebsiteForm(wrapper);
    await wrapper.find("form").trigger("submit");
    await flushPromises();

    expect(wrapper.text()).toContain("Remote copy exists");

    const keepMine = findButton(wrapper, "Keep mine");
    expect(keepMine).toBeTruthy();
    await keepMine!.trigger("click");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("resolve_write_conflict", {
      choice: "keep_mine",
    });
    expect(mockPush).toHaveBeenCalledWith({ name: "entries" });
  });

  it("on an undecryptable conflict, force is gated behind explicit confirmation", async () => {
    const wrapper = await mountPage({
      create_from_preset_secret: () => ({
        kind: "conflict",
        name: "websites/example.com/alice",
        remote_decryptable: false,
      }),
    });

    await fillWebsiteForm(wrapper);
    await wrapper.find("form").trigger("submit");
    await flushPromises();

    const force = wrapper.find(".btn-danger");
    expect((force.element as HTMLButtonElement).disabled).toBe(true);

    await wrapper.find('input[type="checkbox"]').setValue(true);
    expect((force.element as HTMLButtonElement).disabled).toBe(false);
  });

  it("cancel resolves with `cancel` and does not navigate", async () => {
    const wrapper = await mountPage({
      create_from_preset_secret: () => ({
        kind: "conflict",
        name: "websites/example.com/alice",
        remote_decryptable: true,
      }),
      resolve_write_conflict: () => null,
    });

    await fillWebsiteForm(wrapper);
    await wrapper.find("form").trigger("submit");
    await flushPromises();

    const cancel = findButton(wrapper, "Cancel");
    expect(cancel).toBeTruthy();
    await cancel!.trigger("click");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("resolve_write_conflict", {
      choice: "cancel",
    });
    expect(mockPush).not.toHaveBeenCalled();
  });
});
