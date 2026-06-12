// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
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
    // Default: repo not ready (fresh setup)
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === "is_repo_ready") return false;
      return undefined;
    });
  });

  // ── Step 1: Clone ──────────────────────────────────────────────────────

  async function fillStep1(
    wrapper: ReturnType<typeof mount>,
    opts: {
      repoUrl?: string;
      pat?: string;
      sshKey?: string;
      sshPassphrase?: string;
    } = {},
  ) {
    const defaults = {
      repoUrl: "https://github.com/user/passwords.git",
      pat: "",
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
    if (opts.sshKey !== undefined) {
      const sshKeyEl = wrapper.find('textarea[id="ssh-key"]');
      if (sshKeyEl.exists()) {
        await sshKeyEl.setValue(opts.sshKey);
      }
    }
    if (opts.sshPassphrase !== undefined) {
      const sshPassEl = wrapper.find('input[id="ssh-passphrase"]');
      if (sshPassEl.exists()) {
        await sshPassEl.setValue(opts.sshPassphrase);
      }
    }
  }

  async function submitStep1(wrapper: ReturnType<typeof mount>) {
    await wrapper.find("form").trigger("submit.prevent");
    await flushPromises();
  }

  describe("step 1 validation", () => {
    it("shows error when repo URL is empty", async () => {
      const wrapper = mount(SetupPage);
      await flushPromises();
      await fillStep1(wrapper, { repoUrl: "" });
      await submitStep1(wrapper);

      expect(wrapper.find("[role='alert']").text()).toBe(
        "Repository URL is required",
      );
      expect(invoke).not.toHaveBeenCalledWith("clone_repo", expect.anything());
    });

    it("shows error for non-HTTPS URL", async () => {
      const wrapper = mount(SetupPage);
      await flushPromises();
      await fillStep1(wrapper, {
        repoUrl: "http://github.com/user/repo.git",
      });
      await submitStep1(wrapper);

      expect(wrapper.find("[role='alert']").text()).toBe(
        "URL must be HTTPS or SSH format (e.g. git@host:user/repo.git)",
      );
    });
  });

  describe("step 1 clone", () => {
    it("calls clone_repo with correct args for HTTPS", async () => {
      vi.mocked(invoke)
        .mockImplementation(async (cmd: string) => {
          if (cmd === "is_repo_ready") return false;
          return undefined;
        })
        .mockResolvedValueOnce(false) // is_repo_ready
        .mockResolvedValueOnce(undefined); // clone_repo

      const wrapper = mount(SetupPage);
      await flushPromises();
      await fillStep1(wrapper, {
        repoUrl: "https://github.com/user/repo.git",
        pat: "my-token",
      });
      await submitStep1(wrapper);

      expect(invoke).toHaveBeenCalledWith("clone_repo", {
        repoUrl: "https://github.com/user/repo.git",
        pat: "my-token",
        sshKey: null,
        sshPassphrase: null,
      });
    });

    it("passes null for pat when empty", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(false) // is_repo_ready
        .mockResolvedValueOnce(undefined); // clone_repo

      const wrapper = mount(SetupPage);
      await flushPromises();
      await fillStep1(wrapper, { pat: "" });
      await submitStep1(wrapper);

      expect(invoke).toHaveBeenCalledWith(
        "clone_repo",
        expect.objectContaining({ pat: null }),
      );
    });

    it("advances to step 2 on success", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(false) // is_repo_ready
        .mockResolvedValueOnce(undefined); // clone_repo

      const wrapper = mount(SetupPage);
      await flushPromises();
      await fillStep1(wrapper);
      await submitStep1(wrapper);

      // Step 2 should be visible (identity field)
      expect(wrapper.find('textarea[id="identity"]').exists()).toBe(true);
    });

    it("displays error from clone failure", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(false) // is_repo_ready
        .mockRejectedValueOnce({
          code: "CloneFailed",
          message: "Clone failed",
        });

      const wrapper = mount(SetupPage);
      await flushPromises();
      await fillStep1(wrapper);
      await submitStep1(wrapper);

      expect(wrapper.find("[role='alert']").text()).toBe("Clone failed");
    });
  });

  // ── Step 2: Identity ───────────────────────────────────────────────────

  describe("step 2 identity", () => {
    async function mountAtStep2(recipientsList: RecipientInfo[] = []) {
      vi.mocked(invoke)
        .mockResolvedValueOnce(true) // is_repo_ready
        .mockResolvedValueOnce(recipientsList); // list_recipients

      const wrapper = mount(SetupPage);
      await flushPromises();
      return wrapper;
    }

    it("auto-advances to step 2 when repo is ready", async () => {
      const wrapper = await mountAtStep2([]);

      expect(wrapper.find('textarea[id="identity"]').exists()).toBe(true);
      expect(wrapper.find('input[id="repo-url"]').exists()).toBe(false);
    });

    it("fetches and displays recipients", async () => {
      const recipients: RecipientInfo[] = [
        { public_key: "age1abc123", comment: "Alice" },
        { public_key: "age1def456", comment: null },
      ];
      const wrapper = await mountAtStep2(recipients);

      expect(invoke).toHaveBeenCalledWith("list_recipients");
      expect(wrapper.text()).toContain("age1abc123");
      expect(wrapper.text()).toContain("Alice");
      expect(wrapper.text()).toContain("age1def456");
    });

    it("shows message when no recipients found", async () => {
      const wrapper = await mountAtStep2([]);

      expect(wrapper.text()).toContain("No recipients file found");
    });

    it("allows selecting a recipient", async () => {
      const recipients: RecipientInfo[] = [
        { public_key: "age1abc123", comment: "Alice" },
        { public_key: "age1def456", comment: null },
      ];
      const wrapper = await mountAtStep2(recipients);

      // Click on second recipient
      const items = wrapper.findAll(".cursor-pointer");
      await items[1].trigger("click");

      // Second item should be selected (has accent border class)
      expect(items[1].classes()).toContain("border-accent");
    });

    it("auto-selects first recipient when only one exists", async () => {
      const recipients: RecipientInfo[] = [
        { public_key: "age1only", comment: "Only key" },
      ];
      const wrapper = await mountAtStep2(recipients);

      // The single recipient should show as selected
      const item = wrapper.find(".cursor-pointer");
      expect(item.classes()).toContain("border-accent");
    });

    it("calls complete_setup with identity", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(true) // is_repo_ready
        .mockResolvedValueOnce([]) // list_recipients
        .mockResolvedValueOnce(undefined); // complete_setup

      const wrapper = mount(SetupPage);
      await flushPromises();

      await wrapper
        .find('textarea[id="identity"]')
        .setValue("AGE-SECRET-KEY-1abc");
      await wrapper.find("form").trigger("submit.prevent");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("complete_setup", {
        identity: "AGE-SECRET-KEY-1abc",
        passphrase: null,
      });
    });

    it("navigates to entries on success", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(true) // is_repo_ready
        .mockResolvedValueOnce([]) // list_recipients
        .mockResolvedValueOnce(undefined); // complete_setup

      const wrapper = mount(SetupPage);
      await flushPromises();

      await wrapper
        .find('textarea[id="identity"]')
        .setValue("AGE-SECRET-KEY-1abc");
      await wrapper.find("form").trigger("submit.prevent");
      await flushPromises();

      expect(mockPush).toHaveBeenCalledWith({ name: "entries" });
    });

    it("shows error when identity is empty", async () => {
      const wrapper = await mountAtStep2([]);

      await wrapper.find('textarea[id="identity"]').setValue("");
      await wrapper.find("form").trigger("submit.prevent");
      await flushPromises();

      expect(wrapper.find("[role='alert']").text()).toBe(
        "Age identity is required",
      );
    });

    it("shows error when identity format is invalid", async () => {
      const wrapper = await mountAtStep2([]);

      await wrapper.find('textarea[id="identity"]').setValue("not-a-valid-key");
      await wrapper.find("form").trigger("submit.prevent");
      await flushPromises();

      expect(wrapper.find("[role='alert']").text()).toBe(
        "Identity must be an age key (AGE-SECRET-KEY-...) or SSH private key",
      );
    });

    it("shows error when recipients exist but none selected", async () => {
      const recipients: RecipientInfo[] = [
        { public_key: "age1abc123", comment: "Alice" },
        { public_key: "age1def456", comment: null },
      ];
      const wrapper = await mountAtStep2(recipients);

      await wrapper
        .find('textarea[id="identity"]')
        .setValue("AGE-SECRET-KEY-1abc");
      await wrapper.find("form").trigger("submit.prevent");
      await flushPromises();

      expect(wrapper.find("[role='alert']").text()).toBe(
        "Please select a recipient",
      );
    });

    it("displays error from complete_setup failure", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(true) // is_repo_ready
        .mockResolvedValueOnce([]) // list_recipients
        .mockRejectedValueOnce({
          code: "INVALID_IDENTITY",
          message: "Identity does not match any recipient",
        });

      const wrapper = mount(SetupPage);
      await flushPromises();

      await wrapper
        .find('textarea[id="identity"]')
        .setValue("AGE-SECRET-KEY-1abc");
      await wrapper.find("form").trigger("submit.prevent");
      await flushPromises();

      expect(wrapper.find("[role='alert']").text()).toBe(
        "Identity does not match any recipient",
      );
    });
  });

  // ── Back navigation ────────────────────────────────────────────────────

  describe("navigation between steps", () => {
    it("goes back to step 1 when back button clicked", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(true) // is_repo_ready
        .mockResolvedValueOnce([]); // list_recipients

      const wrapper = mount(SetupPage);
      await flushPromises();

      // Should be on step 2
      expect(wrapper.find('textarea[id="identity"]').exists()).toBe(true);

      // Click back
      const backBtn = wrapper.find("button[type='button']");
      await backBtn.trigger("click");
      await flushPromises();

      // Should be on step 1
      expect(wrapper.find('input[id="repo-url"]').exists()).toBe(true);
    });
  });

  // ── SSH URL support (step 1) ──────────────────────────────────────────

  describe("SSH URL support", () => {
    it("shows SSH key field for git@ URL", async () => {
      vi.mocked(invoke).mockResolvedValueOnce(false); // is_repo_ready

      const wrapper = mount(SetupPage);
      await flushPromises();
      await wrapper
        .find('input[id="repo-url"]')
        .setValue("git@github.com:user/repo.git");
      await flushPromises();

      expect(wrapper.find('textarea[id="ssh-key"]').exists()).toBe(true);
      expect(wrapper.find('input[id="ssh-passphrase"]').exists()).toBe(true);
      expect(wrapper.find('input[id="pat"]').exists()).toBe(false);
    });

    it("shows SSH key field for ssh:// URL", async () => {
      vi.mocked(invoke).mockResolvedValueOnce(false); // is_repo_ready

      const wrapper = mount(SetupPage);
      await flushPromises();
      await wrapper
        .find('input[id="repo-url"]')
        .setValue("ssh://git@github.com/user/repo.git");
      await flushPromises();

      expect(wrapper.find('textarea[id="ssh-key"]').exists()).toBe(true);
    });

    it("shows error when SSH key is empty for SSH URL", async () => {
      vi.mocked(invoke).mockResolvedValueOnce(false); // is_repo_ready

      const wrapper = mount(SetupPage);
      await flushPromises();
      await fillStep1(wrapper, {
        repoUrl: "git@github.com:user/repo.git",
        sshKey: "",
      });
      await submitStep1(wrapper);

      expect(wrapper.find("[role='alert']").text()).toBe(
        "SSH private key is required for SSH URLs",
      );
    });

    it("calls clone_repo with SSH args for git@ URL", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(false) // is_repo_ready
        .mockResolvedValueOnce(undefined); // clone_repo

      const wrapper = mount(SetupPage);
      await flushPromises();
      await fillStep1(wrapper, {
        repoUrl: "git@github.com:user/repo.git",
        sshKey: "test-ssh-key",
        sshPassphrase: "mypass",
      });
      await submitStep1(wrapper);

      expect(invoke).toHaveBeenCalledWith("clone_repo", {
        repoUrl: "git@github.com:user/repo.git",
        pat: null,
        sshKey: "test-ssh-key",
        sshPassphrase: "mypass",
      });
    });
  });

  // ── SSH key generation (step 1) ──────────────────────────────────────

  describe("SSH key generation", () => {
    it("shows generate tab when SSH URL and generate tab selected", async () => {
      vi.mocked(invoke).mockResolvedValueOnce(false); // is_repo_ready

      const wrapper = mount(SetupPage);
      await flushPromises();
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

      expect(wrapper.text()).toContain("Generate SSH Key");
      expect(wrapper.find('textarea[id="ssh-key"]').exists()).toBe(false);
    });

    it("generates key and displays public key", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(false) // is_repo_ready
        .mockResolvedValueOnce({
          public_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIgenerated",
          private_key:
            "-----BEGIN OPENSSH PRIVATE KEY-----\ngen\n-----END OPENSSH PRIVATE KEY-----",
        });

      const wrapper = mount(SetupPage);
      await flushPromises();
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
  });
});

// Type used in tests
interface RecipientInfo {
  public_key: string;
  comment: string | null;
}
