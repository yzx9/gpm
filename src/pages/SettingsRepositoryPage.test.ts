// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { mountWithApp } from "@/test/appTestUtils";
import {
  baseDefaults,
  httpsConfig,
  resetOverrides,
  sshConfig,
  type Overrides,
} from "@/test/settingsTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import SettingsRepositoryPage from "./SettingsRepositoryPage.vue";

const { mockPush, mockReplace, mockOnBeforeRouteLeave } = vi.hoisted(() => ({
  mockPush: vi.fn(),
  mockReplace: vi.fn(),
  mockOnBeforeRouteLeave: vi.fn(),
}));

vi.mock("@tauri-apps/api/core");
vi.mock("vue-router", () => ({
  createRouter: vi.fn(),
  createWebHashHistory: vi.fn(),
  onBeforeRouteLeave: mockOnBeforeRouteLeave,
  useRouter: () => ({ push: mockPush, replace: mockReplace, back: vi.fn() }),
  useRoute: () => ({
    params: {},
    query: {},
    name: "",
    path: "/",
    fullPath: "/",
  }),
}));

describe("SettingsRepositoryPage", () => {
  const overrides: Overrides = {};
  const defaults = { ...baseDefaults };

  function when(cmd: string, value: unknown) {
    overrides[cmd] = { value };
  }
  function reject(cmd: string, payload: unknown) {
    overrides[cmd] = { reject: payload };
  }
  function installMock() {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd in overrides) {
        const o = overrides[cmd];
        if (o && o.reject !== undefined) return Promise.reject(o.reject);
        return Promise.resolve(o ? o.value : defaults[cmd]);
      }
      return Promise.resolve(defaults[cmd]);
    });
  }

  beforeEach(() => {
    vi.clearAllMocks();
    resetOverrides(overrides);
    installMock();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  function mountPage() {
    return mountWithApp(SettingsRepositoryPage).wrapper;
  }

  describe("config loading", () => {
    it("calls get_config and get_commit_identity_default on mount", async () => {
      mountPage();
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("get_config");
      expect(invoke).toHaveBeenCalledWith("get_commit_identity_default");
    });

    it("displays repo URL from config", async () => {
      when("get_config", sshConfig);
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).toContain("git@github.com:user/repo.git");
    });

    it("shows SSH Key auth type for SSH config", async () => {
      when("get_config", sshConfig);
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).toContain("Auth: SSH Key");
    });

    it("shows PAT auth type for HTTPS config with token", async () => {
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).toContain("Auth: PAT");
    });

    it("shows None auth type for public HTTPS config", async () => {
      when("get_config", { ...httpsConfig, pat: null });
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).toContain("Auth: None (public)");
    });

    it("shows error when config loading fails", async () => {
      reject("get_config", {
        code: "ConfigError",
        message: "Config not found",
      });
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.find("[role='alert']").text()).toContain(
        "Config not found",
      );
    });

    it("shows loading state before config loads", async () => {
      when("get_config", new Promise(() => {}));
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).toContain("Loading...");
    });
  });

  describe("repository authenticity card", () => {
    it("shows the card and the off-mode hint by default", async () => {
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).toContain("Repository Authenticity");
      expect(wrapper.text()).toContain("No verification.");
      expect(wrapper.text()).toContain("Trusted signing keys (0)");
    });

    it("lists trusted keys and offers removal", async () => {
      when("get_authenticity_config", {
        mode: "audit",
        trusted_keys: [
          {
            public_key: "ssh-ed25519 AAAA",
            fingerprint: "SHA256:abcd",
            label: "Alice",
            added_at_commit: "deadbeef",
          },
        ],
        trusted_gpg_keys: [],
        ignored: [],
      });
      vi.mocked(globalThis.confirm).mockReturnValue(true);
      when("remove_trusted_key", undefined);
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).toContain("SHA256:abcd");
      expect(wrapper.text()).toContain("Alice");

      const removeBtn = wrapper
        .findAll(".btn-copy")
        .find((b) => b.text().includes("Remove"));
      expect(removeBtn).toBeDefined();
      await removeBtn!.trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("remove_trusted_key", {
        fingerprint: "SHA256:abcd",
      });
    });

    it("lists GPG trusted keys and routes their remove to remove_trusted_gpg_key", async () => {
      when("get_authenticity_config", {
        mode: "audit",
        trusted_keys: [],
        trusted_gpg_keys: [
          {
            armored_public_key: "-----BEGIN PGP PUBLIC KEY BLOCK-----",
            fingerprint: "abcdef0123456789",
            label: "Bob GPG",
            added_at_commit: "cafef00d",
          },
        ],
        ignored: [],
      });
      vi.mocked(globalThis.confirm).mockReturnValue(true);
      when("remove_trusted_gpg_key", undefined);
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).toContain("abcdef0123456789");
      expect(wrapper.text()).toContain("GPG");

      const removeBtn = wrapper
        .findAll(".btn-copy")
        .find((b) => b.text().includes("Remove"));
      expect(removeBtn).toBeDefined();
      await removeBtn!.trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("remove_trusted_gpg_key", {
        fingerprint: "abcdef0123456789",
      });
    });

    it("import GPG key file button invokes import_trusted_gpg_key_file", async () => {
      vi.spyOn(globalThis, "prompt").mockReturnValue("Bob");
      when("import_trusted_gpg_key_file", undefined);
      const wrapper = mountPage();
      await flushPromises();

      const importBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Import GPG key file"));
      expect(importBtn).toBeDefined();
      await importBtn!.trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("import_trusted_gpg_key_file", {
        label: "Bob",
      });
    });

    it("shows a notice when a trusted GPG key fails to load", async () => {
      when("get_authenticity_config", {
        mode: "audit",
        trusted_keys: [],
        trusted_gpg_keys: [
          {
            armored_public_key: "broken",
            fingerprint: "deadbeef",
            label: "Stale",
            added_at_commit: "x",
          },
        ],
        ignored: [],
      });
      when("get_gpg_key_parse_warnings", ["Invalid GPG public key: broken"]);
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).toContain("1 trusted GPG key(s) failed to load");
    });

    it("switches verification mode", async () => {
      when("set_verification_mode", "audit");
      const wrapper = mountPage();
      await flushPromises();

      const radios = wrapper.findAll('input[name="verify-mode"]');
      expect(radios.length).toBe(3);
      await radios[1]!.trigger("change"); // audit
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("set_verification_mode", {
        mode: "audit",
      });
    });

    it("saves the commit identity", async () => {
      when("set_commit_identity", {
        ...httpsConfig,
        commit_user_name: "Alice",
        commit_user_email: "alice@example.com",
      });
      const wrapper = mountPage();
      await flushPromises();

      await wrapper.find("#commit-name").setValue("Alice");
      await wrapper.find("#commit-email").setValue("alice@example.com");
      const saveBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Save"));
      expect(saveBtn).toBeDefined();
      await saveBtn!.trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("set_commit_identity", {
        name: "Alice",
        email: "alice@example.com",
      });
    });

    it("opens the history page", async () => {
      const wrapper = mountPage();
      await flushPromises();

      const historyBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("View commit history"));
      expect(historyBtn).toBeDefined();
      await historyBtn!.trigger("click");
      await flushPromises();

      expect(mockPush).toHaveBeenCalledWith({ name: "history" });
    });
  });

  describe("dirty tracking", () => {
    it("marks Commit Identity dirty on edit and clears on Save", async () => {
      when("set_commit_identity", {
        ...httpsConfig,
        commit_user_name: "Alice",
        commit_user_email: "",
      });
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).not.toContain("Unsaved changes");

      await wrapper.find("#commit-name").setValue("Alice");
      expect(wrapper.text()).toContain("Unsaved changes");

      await wrapper
        .findAll("button")
        .find((b) => b.text().includes("Save"))!
        .trigger("click");
      await flushPromises();

      expect(wrapper.text()).not.toContain("Unsaved changes");
      expect(invoke).toHaveBeenCalledWith("set_commit_identity", {
        name: "Alice",
        email: null,
      });
    });
  });

  describe("leave guard", () => {
    function leaveGuard() {
      return mockOnBeforeRouteLeave.mock.calls[0][0] as () => Promise<
        boolean | void
      >;
    }

    it("leaves freely when nothing is dirty", async () => {
      const wrapper = mountPage();
      await flushPromises();

      await expect(leaveGuard()()).resolves.toBe(true);
      expect(wrapper.find('[aria-label="Unsaved changes"]').exists()).toBe(
        false,
      );
    });

    it("discard drops changes and leaves", async () => {
      const wrapper = mountPage();
      await flushPromises();

      await wrapper.find("#commit-name").setValue("Alice");

      const p = leaveGuard()();
      await flushPromises();

      const modal = wrapper.find('[aria-label="Unsaved changes"]');
      expect(modal.exists()).toBe(true);
      await modal
        .findAll("button")
        .find((b) => b.text().includes("Discard"))!
        .trigger("click");

      await expect(p).resolves.toBe(true);
    });

    it("save commits and leaves", async () => {
      when("set_commit_identity", {
        ...httpsConfig,
        commit_user_name: "Alice",
        commit_user_email: "",
      });
      const wrapper = mountPage();
      await flushPromises();

      await wrapper.find("#commit-name").setValue("Alice");

      const p = leaveGuard()();
      await flushPromises();

      const modal = wrapper.find('[aria-label="Unsaved changes"]');
      await modal
        .findAll("button")
        .find((b) => b.text().includes("Save and leave"))!
        .trigger("click");

      await expect(p).resolves.toBe(true);
      expect(invoke).toHaveBeenCalledWith(
        "set_commit_identity",
        expect.objectContaining({ name: "Alice" }),
      );
    });

    it("save failure keeps the user on the page", async () => {
      reject("set_commit_identity", { code: "Err", message: "nope" });
      const wrapper = mountPage();
      await flushPromises();

      await wrapper.find("#commit-name").setValue("Alice");

      const p = leaveGuard()();
      await flushPromises();

      const modal = wrapper.find('[aria-label="Unsaved changes"]');
      await modal
        .findAll("button")
        .find((b) => b.text().includes("Save and leave"))!
        .trigger("click");

      await expect(p).resolves.toBe(false);
      expect(wrapper.find('[aria-label="Unsaved changes"]').exists()).toBe(
        false,
      );
      expect(wrapper.find("[role='alert']").text()).toContain("nope");
    });

    it("keep editing cancels the navigation", async () => {
      const wrapper = mountPage();
      await flushPromises();

      await wrapper.find("#commit-name").setValue("Alice");

      const p = leaveGuard()();
      await flushPromises();

      const modal = wrapper.find('[aria-label="Unsaved changes"]');
      await modal
        .findAll("button")
        .find((b) => b.text().includes("Keep editing"))!
        .trigger("click");

      await expect(p).resolves.toBe(false);
      expect(wrapper.find('[aria-label="Unsaved changes"]').exists()).toBe(
        false,
      );
    });
  });
});
