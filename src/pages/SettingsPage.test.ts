// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
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
  /** Default successful return values per command (order-independent). */
  const defaults: Record<string, unknown> = {
    get_config: httpsConfig,
    get_auth_state: {
      configured: true,
      encrypted: false,
      unlocked: false,
      identity_type: "x25519",
    },
    is_biometric_available: false,
    is_biometric_unlock_enabled: false,
    get_authenticity_config: { mode: "off", trusted_keys: [], ignored: [] },
    get_commit_identity_default: { name: "gpm", email: "gpm@local" },
    get_ssh_public_key: { public_key: "ssh-ed25519 default" },
    export_ssh_private_key: { private_key: "default-private" },
  };

  // Per-command overrides: value to resolve, or `{ reject: payload }` to reject.
  const overrides: Record<string, { value?: unknown; reject?: unknown }> = {};

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
    for (const k of Object.keys(overrides)) delete overrides[k];
    vi.useFakeTimers();
    vi.stubGlobal(
      "navigator",
      Object.assign(navigator, {
        clipboard: {
          writeText: vi.fn().mockResolvedValue(undefined),
        },
      }),
    );
    installMock();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  function mountPage() {
    return mountWithApp(SettingsPage).wrapper;
  }

  describe("config loading", () => {
    it("calls get_config on mount", async () => {
      mountPage();
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("get_config");
      expect(invoke).toHaveBeenCalledWith("get_auth_state");
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

  describe("SSH key management", () => {
    it("shows SSH Key section when SSH is configured", async () => {
      when("get_config", sshConfig);
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).toContain("SSH Key");
      expect(wrapper.text()).toContain("Show Public Key");
      expect(wrapper.text()).toContain("Export Private Key");
    });

    it("hides SSH Key section when HTTPS is configured", async () => {
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).not.toContain("Show Public Key");
      expect(wrapper.text()).not.toContain("Export Private Key");
    });

    it("shows public key when Show Public Key is clicked", async () => {
      when("get_config", sshConfig);
      when("get_ssh_public_key", {
        public_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAItest",
      });
      const wrapper = mountPage();
      await flushPromises();

      const buttons = wrapper.findAll("button");
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
      when("get_config", sshConfig);
      reject("get_ssh_public_key", {
        code: "SSH_KEY_INVALID",
        message: "No SSH key configured",
      });
      const wrapper = mountPage();
      await flushPromises();

      const showPublicBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Show Public Key"));
      await showPublicBtn!.trigger("click");
      await flushPromises();

      expect(wrapper.find("[role='alert']").text()).toContain(
        "No SSH key configured",
      );
    });

    it("does nothing when export private key is cancelled", async () => {
      when("get_config", sshConfig);
      vi.mocked(globalThis.confirm).mockReturnValue(false);
      const wrapper = mountPage();
      await flushPromises();

      const invokeCount = (invoke as ReturnType<typeof vi.fn>).mock.calls
        .length;
      const exportBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Export Private Key"));
      await exportBtn!.trigger("click");
      await flushPromises();

      expect((invoke as ReturnType<typeof vi.fn>).mock.calls.length).toBe(
        invokeCount,
      );
    });

    it("shows private key when export is confirmed", async () => {
      when("get_config", sshConfig);
      when("export_ssh_private_key", {
        private_key:
          "-----BEGIN OPENSSH PRIVATE KEY-----\nexported\n-----END OPENSSH PRIVATE KEY-----",
      });
      vi.mocked(globalThis.confirm).mockReturnValue(true);
      const wrapper = mountPage();
      await flushPromises();

      const exportBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Export Private Key"));
      await exportBtn!.trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("export_ssh_private_key");
      expect(wrapper.text()).toContain("Private key is now visible");
      expect(wrapper.text()).toContain("-----BEGIN OPENSSH PRIVATE KEY-----");
    });

    it("shows error when export_ssh_private_key fails", async () => {
      when("get_config", sshConfig);
      reject("export_ssh_private_key", {
        code: "SSH_KEY_INVALID",
        message: "Invalid key",
      });
      vi.mocked(globalThis.confirm).mockReturnValue(true);
      const wrapper = mountPage();
      await flushPromises();

      const exportBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Export Private Key"));
      await exportBtn!.trigger("click");
      await flushPromises();

      expect(wrapper.find("[role='alert']").text()).toContain("Invalid key");
    });

    it("hides private key when Hide button is clicked", async () => {
      when("get_config", sshConfig);
      when("export_ssh_private_key", { private_key: "secret-key-data" });
      vi.mocked(globalThis.confirm).mockReturnValue(true);
      const wrapper = mountPage();
      await flushPromises();

      const exportBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Export Private Key"));
      await exportBtn!.trigger("click");
      await flushPromises();

      expect(wrapper.text()).toContain("secret-key-data");

      const hideBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Hide Private Key"));
      expect(hideBtn).toBeDefined();
      await hideBtn!.trigger("click");
      await flushPromises();

      expect(wrapper.text()).not.toContain("secret-key-data");
    });

    it("copies public key to clipboard", async () => {
      when("get_config", sshConfig);
      when("get_ssh_public_key", { public_key: "ssh-ed25519 AAAAtest" });
      const { wrapper, toast } = mountWithApp(SettingsPage);
      await flushPromises();

      const showPublicBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Show Public Key"));
      await showPublicBtn!.trigger("click");
      await flushPromises();

      const copyButtons = wrapper.findAll(".btn-copy");
      await copyButtons[0].trigger("click");
      await flushPromises();

      expect(navigator.clipboard.writeText).toHaveBeenCalledWith(
        "ssh-ed25519 AAAAtest",
      );
      expect(
        toast.toasts.value.some((t) =>
          t.message.includes("✓ Copied to clipboard"),
        ),
      ).toBe(true);
    });

    it("auto-clears toast after 3 seconds", async () => {
      when("get_config", sshConfig);
      when("get_ssh_public_key", { public_key: "ssh-ed25519 AAAAtest" });
      const { wrapper, toast } = mountWithApp(SettingsPage);
      await flushPromises();

      const showPublicBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Show Public Key"));
      await showPublicBtn!.trigger("click");
      await flushPromises();

      const copyButtons = wrapper.findAll(".btn-copy");
      await copyButtons[0].trigger("click");
      await flushPromises();

      expect(
        toast.toasts.value.some((t) => t.message.includes("✓ Copied")),
      ).toBe(true);

      vi.advanceTimersByTime(3000);
      await flushPromises();

      expect(toast.toasts.value).toHaveLength(0);
    });
  });

  describe("reset", () => {
    type PageWrapper = ReturnType<typeof mountPage>;

    async function openReset(wrapper: PageWrapper) {
      const dangerBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Reset All Data"));
      await dangerBtn!.trigger("click");
      await flushPromises();
    }

    function modalConfirmBtn(wrapper: PageWrapper) {
      return wrapper
        .find('[role="alertdialog"]')
        .findAll("button")
        .find((b) => b.text().includes("Reset"));
    }

    it("opens a type-RESET modal from the Danger Zone without wiping", async () => {
      const wrapper = mountPage();
      await flushPromises();
      expect(wrapper.find('[role="alertdialog"]').exists()).toBe(false);

      await openReset(wrapper);

      expect(wrapper.find('[role="alertdialog"]').exists()).toBe(true);
      expect(wrapper.text()).toContain("Type RESET to confirm");
      // Opening the modal must NOT touch reset_config.
      expect(invoke).not.toHaveBeenCalledWith("reset_config");
    });

    it("calls reset_config and navigates after typing RESET and confirming", async () => {
      when("reset_config", undefined);
      const wrapper = mountPage();
      await flushPromises();
      await openReset(wrapper);

      await wrapper.find('[role="alertdialog"] input').setValue("RESET");
      await modalConfirmBtn(wrapper)!.trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("reset_config");
      expect(mockPush).toHaveBeenCalledWith({ name: "setup" });
    });

    it("keeps the confirm button disabled until RESET is typed", async () => {
      const wrapper = mountPage();
      await flushPromises();
      await openReset(wrapper);

      await wrapper.find('[role="alertdialog"] input').setValue("RESETT");
      expect(
        (modalConfirmBtn(wrapper)!.element as HTMLButtonElement).disabled,
      ).toBe(true);

      // Correcting the text reactively re-enables the confirm button.
      await wrapper.find('[role="alertdialog"] input').setValue("RESET");
      expect(
        (modalConfirmBtn(wrapper)!.element as HTMLButtonElement).disabled,
      ).toBe(false);

      // Cancel closes the modal without invoking reset_config.
      const cancelBtn = wrapper
        .find('[role="alertdialog"]')
        .findAll("button")
        .find((b) => b.text().includes("Cancel"));
      await cancelBtn!.trigger("click");
      await flushPromises();

      expect(wrapper.find('[role="alertdialog"]').exists()).toBe(false);
      expect(invoke).not.toHaveBeenCalledWith("reset_config");
    });

    it("accepts case-insensitive, padded RESET", async () => {
      when("reset_config", undefined);
      const wrapper = mountPage();
      await flushPromises();
      await openReset(wrapper);

      await wrapper.find('[role="alertdialog"] input').setValue("  reset  ");
      expect(
        (modalConfirmBtn(wrapper)!.element as HTMLButtonElement).disabled,
      ).toBe(false);
      await modalConfirmBtn(wrapper)!.trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("reset_config");
    });

    it("shows error when reset fails", async () => {
      reject("reset_config", { code: "Err", message: "Reset failed" });
      const wrapper = mountPage();
      await flushPromises();
      await openReset(wrapper);

      await wrapper.find('[role="alertdialog"] input').setValue("RESET");
      await modalConfirmBtn(wrapper)!.trigger("click");
      await flushPromises();

      expect(wrapper.find("[role='alert']").text()).toContain("Reset failed");
      // A failed reset closes the modal (does not leave it open for retry).
      expect(wrapper.find('[role="alertdialog"]').exists()).toBe(false);
    });
  });

  describe("navigation", () => {
    it("navigates back to entries when Back button clicked", async () => {
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
      when("is_biometric_available", true);
      when("is_biometric_unlock_enabled", true);
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).not.toContain("Biometric Unlock");
    });

    it("reports unavailable when no biometric is present", async () => {
      when("get_auth_state", encryptedAuth);
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).toContain("Biometric Unlock");
      expect(wrapper.text()).toContain("isn't available on this device");
    });

    it("calls enable_biometric_unlock with the passphrase when enabling", async () => {
      when("get_auth_state", encryptedAuth);
      when("is_biometric_available", true);
      when("is_biometric_unlock_enabled", false);
      when("enable_biometric_unlock", undefined);
      const { wrapper, toast } = mountWithApp(SettingsPage);
      await flushPromises();

      const bioInput = wrapper.find('input[type="password"]');
      await bioInput.setValue("my-pass");
      const enableBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Enable Biometric"));
      expect(enableBtn).toBeDefined();
      await enableBtn!.trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("enable_biometric_unlock", {
        passphrase: "my-pass",
      });
      expect(
        toast.toasts.value.some((t) =>
          t.message.includes("Biometric unlock enabled"),
        ),
      ).toBe(true);
    });

    it("shows an error on a wrong passphrase when enabling", async () => {
      when("get_auth_state", encryptedAuth);
      when("is_biometric_available", true);
      when("is_biometric_unlock_enabled", false);
      reject("enable_biometric_unlock", {
        code: "WRONG_PASSPHRASE",
        message: "wrong",
      });
      const wrapper = mountPage();
      await flushPromises();

      const bioInput = wrapper.find('input[type="password"]');
      await bioInput.setValue("bad");
      const enableBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Enable Biometric"));
      await enableBtn!.trigger("click");
      await flushPromises();

      expect(wrapper.find("[role='alert']").text()).toContain(
        "Wrong passphrase",
      );
    });

    it("calls disable_biometric_unlock when disabling", async () => {
      when("get_auth_state", encryptedAuth);
      when("is_biometric_available", true);
      when("is_biometric_unlock_enabled", true);
      when("disable_biometric_unlock", undefined);
      const wrapper = mountPage();
      await flushPromises();

      const disableBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Disable Biometric"));
      expect(disableBtn).toBeDefined();
      await disableBtn!.trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("disable_biometric_unlock");
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
});
