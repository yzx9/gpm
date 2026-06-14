// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { mount } from "@vue/test-utils";
import { flushPromises } from "@vue/test-utils";
import { invoke } from "@tauri-apps/api/core";
import SettingsPage from "./SettingsPage.vue";

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

const sshConfig = {
  url: "git@github.com:user/repo.git",
  pat: null,
  ssh_key:
    "-----BEGIN OPENSSH PRIVATE KEY-----\ntest\n-----END OPENSSH PRIVATE KEY-----",
  ssh_passphrase: null,
  local_path: "/tmp/repo",
};

const httpsConfig = {
  url: "https://github.com/user/repo.git",
  pat: "ghp_token123",
  ssh_key: null,
  ssh_passphrase: null,
  local_path: "/tmp/repo",
};

describe("SettingsPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
    vi.stubGlobal(
      "navigator",
      Object.assign(navigator, {
        clipboard: {
          writeText: vi.fn().mockResolvedValue(undefined),
        },
      }),
    );
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  const authState = {
    configured: true,
    encrypted: false,
    unlocked: false,
  };

  function mountPage() {
    return mount(SettingsPage);
  }

  describe("config loading", () => {
    it("calls get_config on mount", async () => {
      vi.mocked(invoke).mockResolvedValueOnce(sshConfig).mockResolvedValueOnce({
        configured: true,
        encrypted: false,
        unlocked: false,
      });
      mountPage();
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("get_config");
      expect(invoke).toHaveBeenCalledWith("get_auth_state");
    });

    it("displays repo URL from config", async () => {
      vi.mocked(invoke).mockResolvedValue(sshConfig);
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).toContain("git@github.com:user/repo.git");
    });

    it("shows SSH Key auth type for SSH config", async () => {
      vi.mocked(invoke).mockResolvedValue(sshConfig);
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).toContain("Auth: SSH Key");
    });

    it("shows PAT auth type for HTTPS config with token", async () => {
      vi.mocked(invoke).mockResolvedValue(httpsConfig);
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).toContain("Auth: PAT");
    });

    it("shows None auth type for public HTTPS config", async () => {
      vi.mocked(invoke).mockResolvedValue({ ...httpsConfig, pat: null });
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).toContain("Auth: None (public)");
    });

    it("shows error when config loading fails", async () => {
      vi.mocked(invoke).mockRejectedValue({
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
      vi.mocked(invoke).mockReturnValue(new Promise(() => {}));
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).toContain("Loading...");
    });
  });

  describe("SSH key management", () => {
    it("shows SSH Key section when SSH is configured", async () => {
      vi.mocked(invoke).mockResolvedValue(sshConfig);
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).toContain("SSH Key");
      expect(wrapper.text()).toContain("Show Public Key");
      expect(wrapper.text()).toContain("Export Private Key");
    });

    it("hides SSH Key section when HTTPS is configured", async () => {
      vi.mocked(invoke).mockResolvedValue(httpsConfig);
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).not.toContain("Show Public Key");
      expect(wrapper.text()).not.toContain("Export Private Key");
    });

    it("shows public key when Show Public Key is clicked", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(sshConfig) // get_config
        .mockResolvedValueOnce(authState) // get_auth_state
        .mockResolvedValueOnce(false) // is_biometric_available
        .mockResolvedValueOnce(false) // is_biometric_unlock_enabled
        .mockResolvedValueOnce({
          public_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAItest",
        }); // get_ssh_public_key
      const wrapper = mountPage();
      await flushPromises();

      // Click Show Public Key button (first btn-action)
      const buttons = wrapper.findAll(".btn-action");
      const showPublicBtn = buttons.find((b) =>
        b.text().includes("Show Public Key"),
      );
      expect(showPublicBtn).toBeDefined();
      await showPublicBtn!.trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("get_ssh_public_key");
      expect(wrapper.text()).toContain(
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAItest",
      );
    });

    it("shows error when get_ssh_public_key fails", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(sshConfig)
        .mockResolvedValueOnce(authState)
        .mockResolvedValueOnce(false)
        .mockResolvedValueOnce(false)
        .mockRejectedValueOnce({
          code: "SSH_KEY_INVALID",
          message: "No SSH key configured",
        });
      const wrapper = mountPage();
      await flushPromises();

      const showPublicBtn = wrapper
        .findAll(".btn-action")
        .find((b) => b.text().includes("Show Public Key"));
      await showPublicBtn!.trigger("click");
      await flushPromises();

      expect(wrapper.find("[role='alert']").text()).toContain(
        "No SSH key configured",
      );
    });

    it("does nothing when export private key is cancelled", async () => {
      vi.mocked(invoke).mockResolvedValue(sshConfig);
      vi.mocked(globalThis.confirm).mockReturnValue(false);
      const wrapper = mountPage();
      await flushPromises();

      const invokeCount = (invoke as ReturnType<typeof vi.fn>).mock.calls
        .length;
      const exportBtn = wrapper
        .findAll(".btn-action")
        .find((b) => b.text().includes("Export Private Key"));
      await exportBtn!.trigger("click");
      await flushPromises();

      // No new invoke calls beyond initial get_config
      expect((invoke as ReturnType<typeof vi.fn>).mock.calls.length).toBe(
        invokeCount,
      );
    });

    it("shows private key when export is confirmed", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(sshConfig) // get_config
        .mockResolvedValueOnce(authState) // get_auth_state
        .mockResolvedValueOnce(false) // is_biometric_available
        .mockResolvedValueOnce(false) // is_biometric_unlock_enabled
        .mockResolvedValueOnce({
          private_key:
            "-----BEGIN OPENSSH PRIVATE KEY-----\nexported\n-----END OPENSSH PRIVATE KEY-----",
        }); // export_ssh_private_key
      vi.mocked(globalThis.confirm).mockReturnValue(true);
      const wrapper = mountPage();
      await flushPromises();

      const exportBtn = wrapper
        .findAll(".btn-action")
        .find((b) => b.text().includes("Export Private Key"));
      await exportBtn!.trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("export_ssh_private_key");
      expect(wrapper.text()).toContain("Private key is now visible");
      expect(wrapper.text()).toContain("-----BEGIN OPENSSH PRIVATE KEY-----");
    });

    it("shows error when export_ssh_private_key fails", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(sshConfig)
        .mockResolvedValueOnce(authState)
        .mockResolvedValueOnce(false)
        .mockResolvedValueOnce(false)
        .mockRejectedValueOnce({
          code: "SSH_KEY_INVALID",
          message: "Invalid key",
        });
      vi.mocked(globalThis.confirm).mockReturnValue(true);
      const wrapper = mountPage();
      await flushPromises();

      const exportBtn = wrapper
        .findAll(".btn-action")
        .find((b) => b.text().includes("Export Private Key"));
      await exportBtn!.trigger("click");
      await flushPromises();

      expect(wrapper.find("[role='alert']").text()).toContain("Invalid key");
    });

    it("hides private key when Hide button is clicked", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(sshConfig)
        .mockResolvedValueOnce(authState)
        .mockResolvedValueOnce(false)
        .mockResolvedValueOnce(false)
        .mockResolvedValueOnce({
          private_key: "secret-key-data",
        });
      vi.mocked(globalThis.confirm).mockReturnValue(true);
      const wrapper = mountPage();
      await flushPromises();

      // Show private key first
      const exportBtn = wrapper
        .findAll(".btn-action")
        .find((b) => b.text().includes("Export Private Key"));
      await exportBtn!.trigger("click");
      await flushPromises();

      expect(wrapper.text()).toContain("secret-key-data");

      // Click Hide button
      const hideBtn = wrapper
        .findAll(".btn-action")
        .find((b) => b.text().includes("Hide Private Key"));
      expect(hideBtn).toBeDefined();
      await hideBtn!.trigger("click");
      await flushPromises();

      expect(wrapper.text()).not.toContain("secret-key-data");
    });

    it("copies public key to clipboard", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(sshConfig)
        .mockResolvedValueOnce(authState)
        .mockResolvedValueOnce(false)
        .mockResolvedValueOnce(false)
        .mockResolvedValueOnce({
          public_key: "ssh-ed25519 AAAAtest",
        });
      const wrapper = mountPage();
      await flushPromises();

      // Show public key first
      const showPublicBtn = wrapper
        .findAll(".btn-action")
        .find((b) => b.text().includes("Show Public Key"));
      await showPublicBtn!.trigger("click");
      await flushPromises();

      // Click Copy button next to public key
      const copyButtons = wrapper.findAll(".btn-copy");
      await copyButtons[0].trigger("click");
      await flushPromises();

      expect(navigator.clipboard.writeText).toHaveBeenCalledWith(
        "ssh-ed25519 AAAAtest",
      );
      expect(wrapper.text()).toContain("✓ Copied to clipboard");
    });

    it("auto-clears toast after 3 seconds", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(sshConfig)
        .mockResolvedValueOnce(authState)
        .mockResolvedValueOnce(false)
        .mockResolvedValueOnce(false)
        .mockResolvedValueOnce({
          public_key: "ssh-ed25519 AAAAtest",
        });
      const wrapper = mountPage();
      await flushPromises();

      // Show + copy to trigger toast
      const showPublicBtn = wrapper
        .findAll(".btn-action")
        .find((b) => b.text().includes("Show Public Key"));
      await showPublicBtn!.trigger("click");
      await flushPromises();

      const copyButtons = wrapper.findAll(".btn-copy");
      await copyButtons[0].trigger("click");
      await flushPromises();

      expect(wrapper.text()).toContain("✓ Copied");

      vi.advanceTimersByTime(3000);
      await flushPromises();

      expect(wrapper.text()).not.toContain("✓ Copied");
    });
  });

  describe("reset", () => {
    it("calls reset_config and navigates when confirmed", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(httpsConfig) // get_config
        .mockResolvedValueOnce(authState) // get_auth_state
        .mockResolvedValueOnce(false) // is_biometric_available
        .mockResolvedValueOnce(false) // is_biometric_unlock_enabled
        .mockResolvedValueOnce(undefined); // reset_config
      vi.mocked(globalThis.confirm).mockReturnValue(true);
      const wrapper = mountPage();
      await flushPromises();

      const resetBtn = wrapper
        .findAll(".btn-action")
        .find((b) => b.text().includes("Reset All Data"));
      expect(resetBtn).toBeDefined();
      await resetBtn!.trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("reset_config");
      expect(mockPush).toHaveBeenCalledWith({ name: "setup" });
    });

    it("does nothing when reset is cancelled", async () => {
      vi.mocked(invoke).mockResolvedValue(httpsConfig);
      vi.mocked(globalThis.confirm).mockReturnValue(false);
      const wrapper = mountPage();
      await flushPromises();

      const invokeCount = (invoke as ReturnType<typeof vi.fn>).mock.calls
        .length;
      const resetBtn = wrapper
        .findAll(".btn-action")
        .find((b) => b.text().includes("Reset All Data"));
      await resetBtn!.trigger("click");
      await flushPromises();

      expect((invoke as ReturnType<typeof vi.fn>).mock.calls.length).toBe(
        invokeCount,
      );
    });

    it("shows error when reset fails", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(httpsConfig)
        .mockResolvedValueOnce(authState)
        .mockResolvedValueOnce(false)
        .mockResolvedValueOnce(false)
        .mockRejectedValueOnce({ code: "Err", message: "Reset failed" });
      vi.mocked(globalThis.confirm).mockReturnValue(true);
      const wrapper = mountPage();
      await flushPromises();

      const resetBtn = wrapper
        .findAll(".btn-action")
        .find((b) => b.text().includes("Reset All Data"));
      await resetBtn!.trigger("click");
      await flushPromises();

      expect(wrapper.find("[role='alert']").text()).toContain("Reset failed");
    });
  });

  describe("navigation", () => {
    it("navigates back to entries when Back button clicked", async () => {
      vi.mocked(invoke).mockResolvedValue(httpsConfig);
      const wrapper = mountPage();
      await flushPromises();

      await wrapper
        .find('button[aria-label="Back to entries"]')
        .trigger("click");
      await flushPromises();

      expect(mockPush).toHaveBeenCalledWith({ name: "entries" });
    });
  });

  describe("biometric unlock card", () => {
    // Auth snapshot for an encrypted identity (card is gated on this).
    const encryptedAuth = {
      configured: true,
      encrypted: true,
      unlocked: false,
      identity_type: "x25519",
    };

    it("is hidden when the identity is not encrypted", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(httpsConfig) // get_config
        .mockResolvedValueOnce(authState) // get_auth_state (encrypted: false)
        .mockResolvedValueOnce(true) // is_biometric_available
        .mockResolvedValueOnce(true); // is_biometric_unlock_enabled
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).not.toContain("Biometric Unlock");
    });

    it("reports unavailable when no biometric is present", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(httpsConfig) // get_config
        .mockResolvedValueOnce(encryptedAuth) // get_auth_state
        .mockResolvedValueOnce(false) // is_biometric_available
        .mockResolvedValueOnce(false); // is_biometric_unlock_enabled
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).toContain("Biometric Unlock");
      expect(wrapper.text()).toContain("isn't available on this device");
    });

    it("calls enable_biometric_unlock with the passphrase when enabling", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(httpsConfig) // get_config
        .mockResolvedValueOnce(encryptedAuth) // get_auth_state
        .mockResolvedValueOnce(true) // is_biometric_available
        .mockResolvedValueOnce(false) // is_biometric_unlock_enabled (not yet)
        .mockResolvedValueOnce(undefined); // enable_biometric_unlock
      const wrapper = mountPage();
      await flushPromises();

      const bioInput = wrapper.find('input[type="password"]');
      await bioInput.setValue("my-pass");
      const enableBtn = wrapper
        .findAll(".btn-action")
        .find((b) => b.text().includes("Enable Biometric"));
      expect(enableBtn).toBeDefined();
      await enableBtn!.trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("enable_biometric_unlock", {
        passphrase: "my-pass",
      });
      expect(wrapper.text()).toContain("Biometric unlock enabled");
    });

    it("shows an error on a wrong passphrase when enabling", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(httpsConfig) // get_config
        .mockResolvedValueOnce(encryptedAuth) // get_auth_state
        .mockResolvedValueOnce(true) // is_biometric_available
        .mockResolvedValueOnce(false) // is_biometric_unlock_enabled
        .mockRejectedValueOnce({
          code: "WRONG_PASSPHRASE",
          message: "wrong",
        }); // enable_biometric_unlock
      const wrapper = mountPage();
      await flushPromises();

      const bioInput = wrapper.find('input[type="password"]');
      await bioInput.setValue("bad");
      const enableBtn = wrapper
        .findAll(".btn-action")
        .find((b) => b.text().includes("Enable Biometric"));
      await enableBtn!.trigger("click");
      await flushPromises();

      expect(wrapper.find("[role='alert']").text()).toContain(
        "Wrong passphrase",
      );
    });

    it("calls disable_biometric_unlock when disabling", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(httpsConfig) // get_config
        .mockResolvedValueOnce(encryptedAuth) // get_auth_state
        .mockResolvedValueOnce(true) // is_biometric_available
        .mockResolvedValueOnce(true) // is_biometric_unlock_enabled
        .mockResolvedValueOnce(undefined); // disable_biometric_unlock
      const wrapper = mountPage();
      await flushPromises();

      const disableBtn = wrapper
        .findAll(".btn-action")
        .find((b) => b.text().includes("Disable Biometric"));
      expect(disableBtn).toBeDefined();
      await disableBtn!.trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("disable_biometric_unlock");
    });
  });
});
