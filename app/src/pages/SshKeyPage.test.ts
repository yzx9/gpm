// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import SshKeyPage from "./SshKeyPage.vue";

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
      if (cmd === "get_config")
        return Promise.resolve({
          url: "git@example.com:owner/repo.git",
          ssh_key: "••••",
          ssh_passphrase: null,
          pat: null,
        });
      if (cmd === "generate_ssh_key")
        return Promise.resolve({
          private_key: "-----GENERATED PRIVATE KEY-----",
          public_key: "ssh-ed25519 AAAAGENERATED",
        });
      return Promise.resolve(undefined);
    });
  });

  it("loads the public key on mount and shows it", async () => {
    const w = mountWithApp(SshKeyPage).wrapper;
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith("get_ssh_public_key", {
      repoId: "test-repo",
    });
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
    expect(invoke).toHaveBeenCalledWith("export_ssh_private_key", {
      repoId: "test-repo",
    });
    expect(w.text()).toContain("-----OPENSSH PRIVATE KEY-----");
  });

  it("Back returns to settings (navBack falls to replace at the root)", async () => {
    const w = mountWithApp(SshKeyPage).wrapper;
    await flushPromises();
    await w.find('button[aria-label="Back"]').trigger("click");
    await flushPromises();
    expect(mockReplace).toHaveBeenCalledWith({ name: "settings" });
  });

  it("shows an empty state (not an error) when no SSH key is configured", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_ssh_public_key") return Promise.resolve({ public_key: null });
      if (cmd === "export_ssh_private_key")
        return Promise.resolve({
          private_key: "-----OPENSSH PRIVATE KEY-----",
        });
      return Promise.resolve(undefined);
    });
    const w = mountWithApp(SshKeyPage).wrapper;
    await flushPromises();

    expect(w.text()).toContain("No SSH key is configured");
    expect(w.text()).not.toContain("ssh-ed25519");
  });

  it("holds a screen-capture claim while the replace form holds a draft, and drops it when cleared", async () => {
    const w = mountWithApp(SshKeyPage).wrapper;
    await flushPromises();
    vi.mocked(invoke).mockClear();

    // A pasted key is a plaintext private key on screen — the claim raises
    // FLAG_SECURE for as long as the draft exists.
    await w.get("#ssh-key").setValue("pasted-key");
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith("plugin:screen-secure|set_secure", {
      secure: true,
    });

    await w.get("#ssh-key").setValue("");
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith("plugin:screen-secure|set_secure", {
      secure: false,
    });
  });

  it("Remove key (after confirm) clears the SSH key and returns to settings", async () => {
    const w = mountWithApp(SshKeyPage).wrapper;
    await flushPromises();

    await w
      .findAll("button")
      .find((b) => b.text().includes("Remove Key"))!
      .trigger("click");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("clear_ssh_key", {
      repoId: "test-repo",
    });
    expect(mockPush).toHaveBeenCalledWith({ name: "settingsRepository" });
  });

  // ── Replace / add (probe-then-swap) ────────────────────────────────────

  it("Replace verifies the new pair against the remote, then saves and refreshes the public key", async () => {
    const w = mountWithApp(SshKeyPage).wrapper;
    await flushPromises();
    vi.mocked(invoke).mockClear();

    await w.get("#ssh-key").setValue("pasted-key");
    await w.get("#ssh-passphrase").setValue("pw");
    await w
      .findAll("button")
      .find((b) => b.text().includes("Replace key"))!
      .trigger("click");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("verify_ssh_auth", {
      repoId: "test-repo",
      sshKey: "pasted-key",
      sshPassphrase: "pw",
    });
    expect(invoke).toHaveBeenCalledWith("set_ssh_key", {
      repoId: "test-repo",
      sshKey: "pasted-key",
      sshPassphrase: "pw",
    });
    expect(invoke).toHaveBeenCalledWith("get_ssh_public_key", {
      repoId: "test-repo",
    });
  });

  it("a key that fails verify is NOT saved", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_ssh_public_key")
        return Promise.resolve({ public_key: "ssh-ed25519 AAAA public" });
      if (cmd === "get_config")
        return Promise.resolve({
          url: "git@example.com:owner/repo.git",
          ssh_key: "••••",
          ssh_passphrase: null,
          pat: null,
        });
      if (cmd === "verify_ssh_auth")
        return Promise.reject({
          code: "CLONE_FAILED",
          message: "authentication failed",
        });
      return Promise.resolve(undefined);
    });
    const w = mountWithApp(SshKeyPage).wrapper;
    await flushPromises();

    await w.get("#ssh-key").setValue("bad-key");
    await w
      .findAll("button")
      .find((b) => b.text().includes("Replace key"))!
      .trigger("click");
    await flushPromises();

    const calls = vi.mocked(invoke).mock.calls.map((c) => c[0]);
    expect(calls).not.toContain("set_ssh_key");
    expect(w.text()).toContain("Could not verify the new key");
  });

  it("Generate tab: generate a keypair, then Replace saves it", async () => {
    const w = mountWithApp(SshKeyPage).wrapper;
    await flushPromises();
    vi.mocked(invoke).mockClear();

    await w
      .findAll("label")
      .find((l) => l.text().includes("Generate Key"))!
      .find('input[type="radio"]')
      .trigger("change");
    await flushPromises();
    await w
      .findAll("button")
      .find((b) => b.text().includes("Generate SSH Key"))!
      .trigger("click");
    await flushPromises();
    await w
      .findAll("button")
      .find((b) => b.text().includes("Replace key"))!
      .trigger("click");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("generate_ssh_key", {
      passphrase: null,
    });
    expect(invoke).toHaveBeenCalledWith("set_ssh_key", {
      repoId: "test-repo",
      sshKey: "-----GENERATED PRIVATE KEY-----",
      sshPassphrase: null,
    });
  });

  it("a CONFIG_BUSY save maps to the config-busy message", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_ssh_public_key")
        return Promise.resolve({ public_key: "ssh-ed25519 AAAA public" });
      if (cmd === "get_config")
        return Promise.resolve({
          url: "git@example.com:owner/repo.git",
          ssh_key: "••••",
          ssh_passphrase: null,
          pat: null,
        });
      if (cmd === "set_ssh_key")
        return Promise.reject({
          code: "CONFIG_BUSY",
          message: "config is busy",
        });
      return Promise.resolve(undefined);
    });
    const w = mountWithApp(SshKeyPage).wrapper;
    await flushPromises();

    await w.get("#ssh-key").setValue("pasted-key");
    await w
      .findAll("button")
      .find((b) => b.text().includes("Replace key"))!
      .trigger("click");
    await flushPromises();

    // The same message the PAT page maps CONFIG_BUSY to.
    expect(w.text()).toContain("try again");
  });

  it("the replace section is hidden for an HTTPS remote", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_ssh_public_key")
        return Promise.resolve({ public_key: "ssh-ed25519 AAAA public" });
      if (cmd === "get_config")
        return Promise.resolve({
          url: "https://github.com/owner/repo.git",
          ssh_key: "••••",
          ssh_passphrase: null,
          pat: "ghp_••••abcd",
        });
      return Promise.resolve(undefined);
    });
    const w = mountWithApp(SshKeyPage).wrapper;
    await flushPromises();

    expect(w.find("#ssh-key").exists()).toBe(false);
    expect(w.findAll("button").find((b) => b.text().includes("Replace key"))).toBeUndefined();
  });

  it("the replace section doubles as the add path when no key is configured", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_ssh_public_key") return Promise.resolve({ public_key: null });
      if (cmd === "get_config")
        return Promise.resolve({
          url: "git@example.com:owner/repo.git",
          ssh_key: null,
          ssh_passphrase: null,
          pat: null,
        });
      return Promise.resolve(undefined);
    });
    const w = mountWithApp(SshKeyPage).wrapper;
    await flushPromises();

    expect(w.text()).toContain("No SSH key is configured");
    expect(w.find("#ssh-key").exists()).toBe(true);
    // No-key state — the action reads "Add key", not "Replace key".
    expect(w.text()).toContain("Add key");
    expect(w.text()).not.toContain("Replace key");
    await w.get("#ssh-key").setValue("first-key");
    await w
      .findAll("button")
      .find((b) => b.text().includes("Add key"))!
      .trigger("click");
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith("set_ssh_key", {
      repoId: "test-repo",
      sshKey: "first-key",
      sshPassphrase: null,
    });
  });

  it("a failed config load shows the error, hides the replace form, keeps the public key", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_ssh_public_key")
        return Promise.resolve({ public_key: "ssh-ed25519 AAAA public" });
      if (cmd === "get_config") return Promise.reject({ code: "INTERNAL", message: "boom" });
      return Promise.resolve(undefined);
    });
    const w = mountWithApp(SshKeyPage).wrapper;
    await flushPromises();

    expect(w.text()).toContain("boom");
    expect(w.text()).toContain("ssh-ed25519 AAAA public");
    // An empty URL classifies non-SSH — the replace section stays hidden.
    expect(w.find("#ssh-key").exists()).toBe(false);
  });

  it("a non-busy save failure surfaces the backend message", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_ssh_public_key")
        return Promise.resolve({ public_key: "ssh-ed25519 AAAA public" });
      if (cmd === "get_config")
        return Promise.resolve({
          url: "git@example.com:owner/repo.git",
          ssh_key: "••••",
          ssh_passphrase: null,
          pat: null,
        });
      if (cmd === "set_ssh_key")
        return Promise.reject({
          code: "SSH_KEY_INVALID",
          message: "An SSH key requires an SSH remote URL",
        });
      return Promise.resolve(undefined);
    });
    const w = mountWithApp(SshKeyPage).wrapper;
    await flushPromises();

    await w.get("#ssh-key").setValue("pasted-key");
    await w
      .findAll("button")
      .find((b) => b.text().includes("Replace key"))!
      .trigger("click");
    await flushPromises();

    expect(w.text()).toContain("An SSH key requires an SSH remote URL");
  });

  // ── Lock wipe (issue #20): the gate mask does not unmount the page ─────

  it("a gate re-lock wipes the replace drafts and marks the drafts notice", async () => {
    const m = mountWithApp(SshKeyPage);
    await flushPromises();
    await m.wrapper.get("#ssh-key").setValue("draft-key");

    m.appLock.setAppLocked(true, "idle");
    await flushPromises();

    expect((m.wrapper.find("#ssh-key").element as HTMLTextAreaElement).value).toBe("");
    expect(m.draftsNotice.consume()).toBe(true); // drafts lost → toast fires
  });

  it("a gate re-lock on an untouched form clears nothing and does NOT mark the notice", async () => {
    const m = mountWithApp(SshKeyPage);
    await flushPromises();

    m.appLock.setAppLocked(true, "idle");
    await flushPromises();

    expect(m.draftsNotice.consume()).toBe(false);
  });
});
