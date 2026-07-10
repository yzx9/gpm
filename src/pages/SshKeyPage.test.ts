// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import SshKeyPage from "./SshKeyPage.vue";

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

describe("SshKeyPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_ssh_public_key")
        return Promise.resolve({ public_key: "ssh-ed25519 AAAA public" });
      if (cmd === "export_ssh_private_key")
        return Promise.resolve({
          private_key: "-----OPENSSH PRIVATE KEY-----",
        });
      return Promise.resolve(undefined);
    });
  });

  it("loads the public key on mount and shows it", async () => {
    const w = mountWithApp(SshKeyPage).wrapper;
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith("get_ssh_public_key");
    expect(w.text()).toContain("ssh-ed25519 AAAA public");
  });

  it("Export (after confirm) reveals the private key", async () => {
    const w = mountWithApp(SshKeyPage).wrapper;
    await flushPromises();
    await w
      .findAll("button")
      .find((b) => b.text().includes("Export Private Key"))!
      .trigger("click");
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith("export_ssh_private_key");
    expect(w.text()).toContain("-----OPENSSH PRIVATE KEY-----");
  });

  it("Back returns to settings (navBack falls to replace at the root)", async () => {
    const w = mountWithApp(SshKeyPage).wrapper;
    await flushPromises();
    await w.find('button[aria-label="Back"]').trigger("click");
    await flushPromises();
    expect(mockReplace).toHaveBeenCalledWith({ name: "settings" });
  });
});
