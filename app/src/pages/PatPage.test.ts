// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import PatPage from "./PatPage.vue";

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

const maskedConfig = {
  url: "https://github.com/u/r.git",
  pat: "ghp_••••wxyz",
  ssh_key: null,
  ssh_passphrase: null,
  local_path: "/tmp/repo",
};
const emptyConfig = { ...maskedConfig, pat: null };

describe("PatPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_config") return Promise.resolve(maskedConfig);
      if (cmd === "verify_git_auth") return Promise.resolve(undefined);
      if (cmd === "set_pat") return Promise.resolve(maskedConfig);
      return Promise.resolve(undefined);
    });
  });

  it("shows the masked token preview", async () => {
    const w = mountWithApp(PatPage).wrapper;
    await flushPromises();
    expect(w.text()).toContain("ghp_••••wxyz");
  });

  it("shows the no-token state when none is configured", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_config") return Promise.resolve(emptyConfig);
      return Promise.resolve(undefined);
    });
    const w = mountWithApp(PatPage).wrapper;
    await flushPromises();
    expect(w.text()).toContain("No token is configured");
  });

  it("Replace is disabled until a token is entered", async () => {
    const w = mountWithApp(PatPage).wrapper;
    await flushPromises();
    const btn = w
      .findAll("button")
      .find((b) => b.text().includes("Replace token"))!;
    expect(btn.attributes("disabled")).toBeDefined();
    await w.find("#new-pat").setValue("ghp_newtokenvalue");
    await flushPromises();
    expect(btn.attributes("disabled")).toBeUndefined();
  });

  it("Replace verifies against the remote, then saves", async () => {
    const w = mountWithApp(PatPage).wrapper;
    await flushPromises();
    await w.find("#new-pat").setValue("ghp_newtokenvalue");
    await w
      .findAll("button")
      .find((b) => b.text().includes("Replace token"))!
      .trigger("click");
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith("verify_git_auth", {
      pat: "ghp_newtokenvalue",
    });
    expect(invoke).toHaveBeenCalledWith("set_pat", {
      pat: "ghp_newtokenvalue",
    });
  });

  it("Replace with a token that fails verify does NOT save", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_config") return Promise.resolve(maskedConfig);
      if (cmd === "verify_git_auth")
        return Promise.reject({ code: "CLONE_FAILED", message: "auth failed" });
      return Promise.resolve(undefined);
    });
    const w = mountWithApp(PatPage).wrapper;
    await flushPromises();
    await w.find("#new-pat").setValue("ghp_badtokenvalue12345");
    await w
      .findAll("button")
      .find((b) => b.text().includes("Replace token"))!
      .trigger("click");
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith("verify_git_auth", {
      pat: "ghp_badtokenvalue12345",
    });
    expect(invoke).not.toHaveBeenCalledWith("set_pat", expect.anything());
    expect(w.text()).toContain("Couldn't authenticate");
  });

  it("Clear (after confirm) removes the token", async () => {
    const w = mountWithApp(PatPage).wrapper;
    await flushPromises();
    await w
      .findAll("button")
      .find((b) => b.text().includes("Clear token"))!
      .trigger("click");
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith("set_pat", { pat: null });
  });
});
