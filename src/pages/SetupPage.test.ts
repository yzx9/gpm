// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount } from "@vue/test-utils";
import { flushPromises } from "@vue/test-utils";
import { invoke } from "@tauri-apps/api/core";
import SetupPage from "./SetupPage.vue";

const { mockPush } = vi.hoisted(() => ({
  mockPush: vi.fn(),
}));

vi.mock("@tauri-apps/api/core");
vi.mock("vue-router", () => ({
  createRouter: vi.fn(),
  createWebHashHistory: vi.fn(),
  useRouter: () => ({
    push: mockPush,
    replace: vi.fn(),
    back: vi.fn(),
  }),
  useRoute: () => ({
    params: {},
    query: {},
    name: "",
    path: "/",
    fullPath: "/",
  }),
}));

describe("SetupPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  async function fillForm(
    wrapper: ReturnType<typeof mount>,
    opts: {
      repoUrl?: string;
      pat?: string;
      sshKey?: string;
      sshPassphrase?: string;
      identity?: string;
    } = {},
  ) {
    const defaults = {
      repoUrl: "https://github.com/user/passwords.git",
      pat: "",
      identity: "AGE-SECRET-KEY-1abc123def456",
    };
    const vals = { ...defaults, ...opts };

    if (vals.repoUrl !== undefined) {
      await wrapper.find('input[id="repo-url"]').setValue(vals.repoUrl);
    }
    if (vals.pat !== undefined) {
      const patEl = wrapper.find('input[id="pat"]');
      if (patEl.exists()) {
        await patEl.setValue(vals.pat);
      }
    }
    if (vals.sshKey !== undefined) {
      const sshKeyEl = wrapper.find('textarea[id="ssh-key"]');
      if (sshKeyEl.exists()) {
        await sshKeyEl.setValue(vals.sshKey);
      }
    }
    if (vals.sshPassphrase !== undefined) {
      const sshPassEl = wrapper.find('input[id="ssh-passphrase"]');
      if (sshPassEl.exists()) {
        await sshPassEl.setValue(vals.sshPassphrase);
      }
    }
    if (vals.identity !== undefined) {
      await wrapper.find('textarea[id="identity"]').setValue(vals.identity);
    }
  }

  async function submitForm(wrapper: ReturnType<typeof mount>) {
    await wrapper.find("form").trigger("submit.prevent");
    await flushPromises();
  }

  describe("validation", () => {
    it("shows error when repo URL is empty", async () => {
      const wrapper = mount(SetupPage);
      await fillForm(wrapper, { repoUrl: "" });
      await submitForm(wrapper);

      expect(wrapper.find("[role='alert']").text()).toBe(
        "Repository URL is required",
      );
      expect(invoke).not.toHaveBeenCalled();
    });

    it("shows error for non-HTTPS URL", async () => {
      const wrapper = mount(SetupPage);
      await fillForm(wrapper, { repoUrl: "http://github.com/user/repo.git" });
      await submitForm(wrapper);

      expect(wrapper.find("[role='alert']").text()).toBe(
        "URL must be HTTPS or SSH format (e.g. git@host:user/repo.git)",
      );
    });

    it("shows error when identity is empty", async () => {
      const wrapper = mount(SetupPage);
      await fillForm(wrapper, {
        repoUrl: "https://github.com/user/repo.git",
        identity: "",
      });
      await submitForm(wrapper);

      expect(wrapper.find("[role='alert']").text()).toBe(
        "Age identity is required",
      );
    });

    it("shows error when identity lacks AGE-SECRET-KEY- prefix", async () => {
      const wrapper = mount(SetupPage);
      await fillForm(wrapper, { identity: "not-a-valid-key" });
      await submitForm(wrapper);

      expect(wrapper.find("[role='alert']").text()).toBe(
        "Identity must start with AGE-SECRET-KEY-...",
      );
    });
  });

  describe("successful submission", () => {
    it("calls invoke with correct args", async () => {
      vi.mocked(invoke).mockResolvedValue(undefined);
      const wrapper = mount(SetupPage);
      await fillForm(wrapper, {
        repoUrl: "https://github.com/user/repo.git",
        pat: "my-token",
        identity: "AGE-SECRET-KEY-1abc",
      });
      await submitForm(wrapper);

      expect(invoke).toHaveBeenCalledWith("setup", {
        repoUrl: "https://github.com/user/repo.git",
        pat: "my-token",
        sshKey: null,
        sshPassphrase: null,
        identity: "AGE-SECRET-KEY-1abc",
      });
    });

    it("passes null for pat when empty", async () => {
      vi.mocked(invoke).mockResolvedValue(undefined);
      const wrapper = mount(SetupPage);
      await fillForm(wrapper, { pat: "" });
      await submitForm(wrapper);

      expect(invoke).toHaveBeenCalledWith(
        "setup",
        expect.objectContaining({ pat: null }),
      );
    });

    it("navigates to entries on success", async () => {
      vi.mocked(invoke).mockResolvedValue(undefined);
      const wrapper = mount(SetupPage);
      await fillForm(wrapper);
      await submitForm(wrapper);

      expect(mockPush).toHaveBeenCalledWith({ name: "entries" });
    });
  });

  describe("error handling", () => {
    it("displays error from AppError", async () => {
      vi.mocked(invoke).mockRejectedValue({
        code: "SetupFailed",
        message: "Clone failed",
      });
      const wrapper = mount(SetupPage);
      await fillForm(wrapper);
      await submitForm(wrapper);

      expect(wrapper.find("[role='alert']").text()).toBe("Clone failed");
    });

    it("falls back to 'Setup failed' on unknown error", async () => {
      vi.mocked(invoke).mockRejectedValue(null);
      const wrapper = mount(SetupPage);
      await fillForm(wrapper);
      await submitForm(wrapper);

      expect(wrapper.find("[role='alert']").text()).toBe("Setup failed");
    });
  });

  describe("SSH URL support", () => {
    it("shows SSH key field for git@ URL", async () => {
      const wrapper = mount(SetupPage);
      await wrapper
        .find('input[id="repo-url"]')
        .setValue("git@github.com:user/repo.git");
      await flushPromises();

      expect(wrapper.find('textarea[id="ssh-key"]').exists()).toBe(true);
      expect(wrapper.find('input[id="ssh-passphrase"]').exists()).toBe(true);
      expect(wrapper.find('input[id="pat"]').exists()).toBe(false);
    });

    it("shows SSH key field for ssh:// URL", async () => {
      const wrapper = mount(SetupPage);
      await wrapper
        .find('input[id="repo-url"]')
        .setValue("ssh://git@github.com/user/repo.git");
      await flushPromises();

      expect(wrapper.find('textarea[id="ssh-key"]').exists()).toBe(true);
      expect(wrapper.find('input[id="ssh-passphrase"]').exists()).toBe(true);
      expect(wrapper.find('input[id="pat"]').exists()).toBe(false);
    });

    it("shows PAT field for HTTPS URL", async () => {
      const wrapper = mount(SetupPage);
      await wrapper
        .find('input[id="repo-url"]')
        .setValue("https://github.com/user/repo.git");
      await flushPromises();

      expect(wrapper.find('input[id="pat"]').exists()).toBe(true);
      expect(wrapper.find('textarea[id="ssh-key"]').exists()).toBe(false);
      expect(wrapper.find('input[id="ssh-passphrase"]').exists()).toBe(false);
    });

    it("shows error when SSH key is empty for SSH URL", async () => {
      const wrapper = mount(SetupPage);
      await fillForm(wrapper, {
        repoUrl: "git@github.com:user/repo.git",
        sshKey: "",
        identity: "AGE-SECRET-KEY-1abc",
      });
      await submitForm(wrapper);

      expect(wrapper.find("[role='alert']").text()).toBe(
        "SSH private key is required for SSH URLs",
      );
      expect(invoke).not.toHaveBeenCalled();
    });

    it("accepts git@ SSH URL without error", async () => {
      vi.mocked(invoke).mockResolvedValue(undefined);
      const wrapper = mount(SetupPage);
      await fillForm(wrapper, {
        repoUrl: "git@github.com:user/repo.git",
        sshKey:
          "-----BEGIN OPENSSH PRIVATE KEY-----\ntest\n-----END OPENSSH PRIVATE KEY-----",
        sshPassphrase: "mypass",
        identity: "AGE-SECRET-KEY-1abc",
      });
      await submitForm(wrapper);

      expect(wrapper.find("[role='alert']").exists()).toBe(false);
    });

    it("calls invoke with SSH args for git@ URL", async () => {
      vi.mocked(invoke).mockResolvedValue(undefined);
      const wrapper = mount(SetupPage);
      await fillForm(wrapper, {
        repoUrl: "git@github.com:user/repo.git",
        sshKey: "test-ssh-key",
        sshPassphrase: "mypass",
        identity: "AGE-SECRET-KEY-1abc",
      });
      await submitForm(wrapper);

      expect(invoke).toHaveBeenCalledWith("setup", {
        repoUrl: "git@github.com:user/repo.git",
        pat: null,
        sshKey: "test-ssh-key",
        sshPassphrase: "mypass",
        identity: "AGE-SECRET-KEY-1abc",
      });
    });

    it("calls invoke with null pat for SSH URL", async () => {
      vi.mocked(invoke).mockResolvedValue(undefined);
      const wrapper = mount(SetupPage);
      await fillForm(wrapper, {
        repoUrl: "git@github.com:user/repo.git",
        sshKey: "test-key",
        identity: "AGE-SECRET-KEY-1abc",
      });
      await submitForm(wrapper);

      expect(invoke).toHaveBeenCalledWith(
        "setup",
        expect.objectContaining({ pat: null, sshKey: "test-key" }),
      );
    });

    it("passes null passphrase when empty for SSH URL", async () => {
      vi.mocked(invoke).mockResolvedValue(undefined);
      const wrapper = mount(SetupPage);
      await fillForm(wrapper, {
        repoUrl: "git@github.com:user/repo.git",
        sshKey: "test-key",
        sshPassphrase: "",
        identity: "AGE-SECRET-KEY-1abc",
      });
      await submitForm(wrapper);

      expect(invoke).toHaveBeenCalledWith(
        "setup",
        expect.objectContaining({ sshPassphrase: null }),
      );
    });
  });

  describe("SSH key generation", () => {
    it("shows generate tab when SSH URL and generate tab selected", async () => {
      const wrapper = mount(SetupPage);
      await wrapper
        .find('input[id="repo-url"]')
        .setValue("git@github.com:user/repo.git");
      await flushPromises();

      // Click "Generate Key" tab
      const tabs = wrapper.findAll("button[type='button']");
      const genTab = tabs.find((b) => b.text().includes("Generate Key"));
      expect(genTab).toBeDefined();
      await genTab!.trigger("click");
      await flushPromises();

      // Should show generate button, not paste textarea
      expect(wrapper.text()).toContain("Generate SSH Key");
      expect(wrapper.find('textarea[id="ssh-key"]').exists()).toBe(false);
    });

    it("generates key and displays public key", async () => {
      vi.mocked(invoke).mockResolvedValue({
        public_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIgenerated",
        private_key:
          "-----BEGIN OPENSSH PRIVATE KEY-----\ngen\n-----END OPENSSH PRIVATE KEY-----",
      });
      const wrapper = mount(SetupPage);
      await wrapper
        .find('input[id="repo-url"]')
        .setValue("git@github.com:user/repo.git");
      await flushPromises();

      // Switch to Generate tab
      const tabs = wrapper.findAll("button[type='button']");
      const genTab = tabs.find((b) => b.text().includes("Generate Key"));
      await genTab!.trigger("click");
      await flushPromises();

      // Click generate button
      const genButton = wrapper
        .findAll("button[type='button']")
        .find((b) => b.text().includes("Generate SSH Key"));
      expect(genButton).toBeDefined();
      await genButton!.trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("generate_ssh_key", {
        passphrase: null,
      });
      expect(wrapper.text()).toContain(
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIgenerated",
      );
    });

    it("shows error when key generation fails", async () => {
      vi.mocked(invoke).mockRejectedValue({
        code: "SSH_KEY_INVALID",
        message: "Key generation failed",
      });
      const wrapper = mount(SetupPage);
      await wrapper
        .find('input[id="repo-url"]')
        .setValue("git@github.com:user/repo.git");
      await flushPromises();

      // Switch to Generate tab and click generate
      const tabs = wrapper.findAll("button[type='button']");
      const genTab = tabs.find((b) => b.text().includes("Generate Key"));
      await genTab!.trigger("click");
      await flushPromises();

      const genButton = wrapper
        .findAll("button[type='button']")
        .find((b) => b.text().includes("Generate SSH Key"));
      await genButton!.trigger("click");
      await flushPromises();

      expect(wrapper.find("[role='alert']").text()).toContain(
        "Key generation failed",
      );
    });

    it("can submit setup after generating key", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce({
          public_key: "ssh-ed25519 AAAAtest",
          private_key: "generated-private-key",
        }) // generate_ssh_key
        .mockResolvedValueOnce(undefined); // setup
      const wrapper = mount(SetupPage);
      await wrapper
        .find('input[id="repo-url"]')
        .setValue("git@github.com:user/repo.git");
      await flushPromises();

      // Generate key
      const tabs = wrapper.findAll("button[type='button']");
      const genTab = tabs.find((b) => b.text().includes("Generate Key"));
      await genTab!.trigger("click");
      await flushPromises();

      const genButton = wrapper
        .findAll("button[type='button']")
        .find((b) => b.text().includes("Generate SSH Key"));
      await genButton!.trigger("click");
      await flushPromises();

      // Fill identity and submit
      await wrapper
        .find('textarea[id="identity"]')
        .setValue("AGE-SECRET-KEY-1abc");
      await submitForm(wrapper);

      expect(invoke).toHaveBeenCalledWith("setup", {
        repoUrl: "git@github.com:user/repo.git",
        pat: null,
        sshKey: "generated-private-key",
        sshPassphrase: null,
        identity: "AGE-SECRET-KEY-1abc",
      });
    });

    it("passes passphrase to generate_ssh_key", async () => {
      vi.mocked(invoke).mockResolvedValue({
        public_key: "ssh-ed25519 AAAAtest",
        private_key: "encrypted-key",
      });
      const wrapper = mount(SetupPage);
      await wrapper
        .find('input[id="repo-url"]')
        .setValue("git@github.com:user/repo.git");
      await flushPromises();

      // Switch to generate tab
      const tabs = wrapper.findAll("button[type='button']");
      const genTab = tabs.find((b) => b.text().includes("Generate Key"));
      await genTab!.trigger("click");
      await flushPromises();

      // Set passphrase in generate tab
      const passphraseInput = wrapper.find('input[id="ssh-gen-passphrase"]');
      expect(passphraseInput.exists()).toBe(true);
      await passphraseInput.setValue("my-passphrase");

      // Click generate
      const genButton = wrapper
        .findAll("button[type='button']")
        .find((b) => b.text().includes("Generate SSH Key"));
      await genButton!.trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("generate_ssh_key", {
        passphrase: "my-passphrase",
      });
    });
  });
});
