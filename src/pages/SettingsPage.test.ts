// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import BaseSegmentedControl from "@/components/base/BaseSegmentedControl.vue";
import { setLocale } from "@/i18n";
import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import SettingsPage from "./SettingsPage.vue";

const { mockPush, mockReplace, mockOnBeforeRouteLeave } = vi.hoisted(() => ({
  mockPush: vi.fn(),
  // Mimic vue-router's pipeline: invoking replace runs the registered leave guard
  // so tests can observe whether the reset bypass short-circuits before the
  // unsaved-changes modal opens. Resolves to undefined (a clean navigation).
  mockReplace: vi.fn(() => {
    const calls = mockOnBeforeRouteLeave.mock.calls;
    // prettier-ignore
    const guard = calls[calls.length - 1]?.[0] as
      (() => Promise<unknown>) | undefined;
    void guard?.();
    return Promise.resolve();
  }),
  mockOnBeforeRouteLeave: vi.fn(),
}));

vi.mock("@tauri-apps/api/core");

// Stub @/i18n so the language-picker tests don't mutate the real i18n singleton
// (SettingsPage imports setLocale/normalizeSupported directly from it; `t()`
// still comes from the test plugin in setup.ts, which is unaffected).
vi.mock("@/i18n", async (importOriginal) => {
  // Preserve the real `i18n` instance (and loadBundle etc.) — `@/i18n/native`
  // imports it for the prompt-text builders — while stubbing the two locale
  // functions this test drives.
  const actual = await importOriginal<typeof import("@/i18n")>();
  return {
    ...actual,
    setLocale: vi.fn().mockResolvedValue(undefined),
    normalizeSupported: vi.fn((tag: string) => tag),
  };
});
vi.mock("vue-router", () => ({
  createRouter: vi.fn(),
  createWebHashHistory: vi.fn(),
  onBeforeRouteLeave: mockOnBeforeRouteLeave,
  useRouter: () => ({
    push: mockPush,
    replace: mockReplace,
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
    get_authenticity_config: {
      mode: "off",
      trusted_keys: [],
      trusted_gpg_keys: [],
      ignored: [],
    },
    get_gpg_key_parse_warnings: [],
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

  describe("identity passphrase", () => {
    it("set passphrase: blocks Encrypt until the unrecoverable ack is checked", async () => {
      const wrapper = mountPage();
      await flushPromises();
      const openBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Set Passphrase"))!;
      await openBtn.trigger("click");
      await flushPromises();
      const modal = wrapper.find('[role="dialog"]');
      const modalBtn = (text: string) =>
        modal.findAll("button").find((b) => b.text().includes(text))!;

      await modal.find('input[id="pp-new"]').setValue("secret");
      await modal.find('input[id="pp-new-confirm"]').setValue("secret");

      // Ack is shown, unchecked → Encrypt disabled; clicking is a no-op.
      const ack = modal.find('input[type="checkbox"]');
      expect(ack.exists()).toBe(true);
      expect((ack.element as HTMLInputElement).checked).toBe(false);
      expect(
        (modalBtn("Encrypt Identity").element as HTMLButtonElement).disabled,
      ).toBe(true);
      await modalBtn("Encrypt Identity").trigger("click");
      await flushPromises();
      expect(invoke).not.toHaveBeenCalledWith(
        "set_passphrase",
        expect.anything(),
      );

      // Acknowledge → Encrypt enables and set_passphrase fires.
      await ack.setValue(true);
      when("set_passphrase", { ok: true });
      await modalBtn("Encrypt Identity").trigger("click");
      await flushPromises();
      expect(invoke).toHaveBeenCalledWith("set_passphrase", {
        passphrase: "secret",
      });
    });

    it("set passphrase: editing the passphrase after acking forces a re-ack", async () => {
      const wrapper = mountPage();
      await flushPromises();
      const openBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Set Passphrase"))!;
      await openBtn.trigger("click");
      await flushPromises();
      const modal = wrapper.find('[role="dialog"]');
      const modalBtn = (text: string) =>
        modal.findAll("button").find((b) => b.text().includes(text))!;
      await modal.find('input[id="pp-new"]').setValue("secret");
      await modal.find('input[id="pp-new-confirm"]').setValue("secret");
      await modal.find('input[type="checkbox"]').setValue(true);
      expect(
        (modalBtn("Encrypt Identity").element as HTMLButtonElement).disabled,
      ).toBe(false);

      // Editing the passphrase after acking invalidates the ack → re-gated.
      await modal.find('input[id="pp-new"]').setValue("changed");
      await modal.find('input[id="pp-new-confirm"]').setValue("changed");
      expect(
        (modal.find('input[type="checkbox"]').element as HTMLInputElement)
          .checked,
      ).toBe(false);
      expect(
        (modalBtn("Encrypt Identity").element as HTMLButtonElement).disabled,
      ).toBe(true);
    });

    it("set passphrase: blocks encrypt when the confirm does not match", async () => {
      const wrapper = mountPage();
      await flushPromises();
      const openBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Set Passphrase"))!;
      await openBtn.trigger("click");
      await flushPromises();
      const modal = wrapper.find('[role="dialog"]');
      const modalBtn = (text: string) =>
        modal.findAll("button").find((b) => b.text().includes(text))!;

      await modal.find('input[id="pp-new"]').setValue("secret");
      await modal.find('input[id="pp-new-confirm"]').setValue("different");
      // Ack first so the mismatch path is what blocks submit.
      await modal.find('input[type="checkbox"]').setValue(true);
      await modalBtn("Encrypt Identity").trigger("click");
      await flushPromises();

      expect(invoke).not.toHaveBeenCalledWith(
        "set_passphrase",
        expect.anything(),
      );
      expect(wrapper.text()).toContain("Passphrases do not match");
    });

    it("change passphrase: submit is gated on the unrecoverable ack too", async () => {
      when("get_auth_state", {
        configured: true,
        encrypted: true,
        unlocked: true,
        identity_type: "x25519",
      });
      const wrapper = mountPage();
      await flushPromises();
      const openBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Change Passphrase"))!;
      await openBtn.trigger("click");
      await flushPromises();
      const modal = wrapper.find('[role="dialog"]');
      const modalBtn = (text: string) =>
        modal.findAll("button").find((b) => b.text().includes(text))!;

      await modal.find('input[id="pp-current"]').setValue("old-pass");
      await modal.find('input[id="pp-new"]').setValue("new-pass");
      await modal.find('input[id="pp-new-confirm"]').setValue("new-pass");

      const ack = modal.find('input[type="checkbox"]');
      // "Change Passphrase" appears on both the card and the modal submit —
      // assert on the modal-scoped submit button.
      expect(
        (modalBtn("Change Passphrase").element as HTMLButtonElement).disabled,
      ).toBe(true);
      await ack.setValue(true);
      when("change_passphrase", { ok: true });
      await modalBtn("Change Passphrase").trigger("click");
      await flushPromises();
      expect(invoke).toHaveBeenCalledWith("change_passphrase", {
        oldPassphrase: "old-pass",
        newPassphrase: "new-pass",
      });
    });

    it("enable-biometric modal does not show the unrecoverable ack", async () => {
      when("get_auth_state", {
        configured: true,
        encrypted: true,
        unlocked: true,
        identity_type: "x25519",
      });
      when("is_biometric_available", true);
      const wrapper = mountPage();
      await flushPromises();
      const openBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Enable Biometric"))!;
      await openBtn.trigger("click");
      await flushPromises();

      // No new passphrase is being established → no unrecoverable ack.
      const modal = wrapper.find('[role="dialog"]');
      expect(modal.text()).not.toContain("cannot be recovered");
      expect(modal.find('input[type="checkbox"]').exists()).toBe(false);
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
      expect(mockReplace).toHaveBeenCalledWith({ name: "setup" });
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

    it("reset bypasses the leave guard even with dirty edits", async () => {
      when("reset_config", undefined);
      const wrapper = mountPage();
      await flushPromises();

      // Dirty the commit-identity form so the guard WOULD prompt if it ran.
      await wrapper.find("#commit-name").setValue("Alice");

      // Open the reset modal, type RESET, confirm.
      await openReset(wrapper);
      await wrapper.find('[role="alertdialog"] input').setValue("RESET");
      await modalConfirmBtn(wrapper)!.trigger("click");
      await flushPromises();

      // Reset navigates via replace, and the unsaved-changes modal never opened
      // (the isResetting gate bypassed the guard before promptUnsaved ran).
      expect(mockReplace).toHaveBeenCalledWith({ name: "setup" });
      expect(wrapper.find('[aria-label="Unsaved changes"]').exists()).toBe(
        false,
      );
    });

    it("the reset gate covers a navigation during the pending wipe", async () => {
      // Hold reset_config pending so a leave-nav can fire during the wipe window
      // (the isResetting gate must cover the whole reset: back on the reset modal
      // is now handled by BaseModalShell, but a nav tap — or back during the
      // listener's async-registration window — could still trip the guard).
      let resolveReset!: () => void;
      when(
        "reset_config",
        new Promise<void>((r) => {
          resolveReset = r;
        }),
      );
      const wrapper = mountPage();
      await flushPromises();

      // Dirty the form, open reset, confirm — doReset is now awaiting apiResetConfig
      // with the isResetting gate already armed (before the wipe).
      await wrapper.find("#commit-name").setValue("Alice");
      await openReset(wrapper);
      await wrapper.find('[role="alertdialog"] input').setValue("RESET");
      await modalConfirmBtn(wrapper)!.trigger("click");
      await flushPromises();

      // A leave-navigation firing mid-wipe is bypassed, so the unsaved-changes modal
      // does not appear.
      const guard = mockOnBeforeRouteLeave.mock.calls[0]?.[0] as () => Promise<
        boolean | void
      >;
      await expect(guard()).resolves.toBe(true);
      expect(wrapper.find('[aria-label="Unsaved changes"]').exists()).toBe(
        false,
      );

      // Release the wipe; doReset finishes and navigates to setup.
      resolveReset();
      await flushPromises();
      expect(mockReplace).toHaveBeenCalledWith({ name: "setup" });
    });
  });

  describe("navigation", () => {
    it("navigates back to entries when Back button clicked", async () => {
      const wrapper = mountPage();
      await flushPromises();

      await wrapper.find('button[aria-label="Back"]').trigger("click");
      await flushPromises();

      expect(mockReplace).toHaveBeenCalledWith({ name: "entries" });
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

      // Card trigger opens the shared passphrase modal.
      const enableBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Enable Biometric"));
      await enableBtn!.trigger("click");
      await flushPromises();

      const modal = wrapper.find('[role="dialog"]');
      expect(modal.exists()).toBe(true);
      await modal.find("#pp-current").setValue("my-pass");
      await modal
        .findAll("button")
        .find((b) => b.text().includes("Enable Biometric"))!
        .trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith(
        "enable_biometric_unlock",
        expect.objectContaining({ passphrase: "my-pass" }),
      );
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

      const enableBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Enable Biometric"));
      await enableBtn!.trigger("click");
      await flushPromises();

      const modal = wrapper.find('[role="dialog"]');
      await modal.find("#pp-current").setValue("bad");
      await modal
        .findAll("button")
        .find((b) => b.text().includes("Enable Biometric"))!
        .trigger("click");
      await flushPromises();

      // Wrong passphrase keeps the modal open with the error visible.
      expect(wrapper.find('[role="dialog"]').exists()).toBe(true);
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

      // GPG rows are badged and the hex fingerprint renders alongside SSH ones.
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

      // The silent Verified→UnverifiedSignature downgrade must be visible.
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

  describe("passphrase modal", () => {
    it("cancel wipes the typed passphrase", async () => {
      const wrapper = mountPage();
      await flushPromises();

      // Default auth is unencrypted, so the Set Passphrase trigger is visible.
      const setBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Set Passphrase"));
      await setBtn!.trigger("click");
      await flushPromises();

      const modal = wrapper.find('[role="dialog"]');
      expect(modal.exists()).toBe(true);
      await modal.find("#pp-new").setValue("secret");
      // The ack is part of the modal state and must be wiped on close too.
      await modal.find('input[type="checkbox"]').setValue(true);
      await modal
        .findAll("button")
        .find((b) => b.text().includes("Cancel"))!
        .trigger("click");
      await flushPromises();

      expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
      expect(invoke).not.toHaveBeenCalledWith(
        "set_passphrase",
        expect.anything(),
      );

      // Re-open: the field AND the ack were wiped on close, not retained.
      await setBtn!.trigger("click");
      await flushPromises();
      expect((wrapper.find("#pp-new").element as HTMLInputElement).value).toBe(
        "",
      );
      expect(
        (
          wrapper.find('[role="dialog"]').find('input[type="checkbox"]')
            .element as HTMLInputElement
        ).checked,
      ).toBe(false);
    });

    it("backdrop dismisses without invoking", async () => {
      const wrapper = mountPage();
      await flushPromises();

      const setBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Set Passphrase"));
      await setBtn!.trigger("click");
      await flushPromises();

      // BaseModalShell emits `close` on a backdrop (click.self) click.
      await wrapper.find('[role="dialog"]').trigger("click");
      await flushPromises();

      expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
      expect(invoke).not.toHaveBeenCalledWith(
        "set_passphrase",
        expect.anything(),
      );
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

    it("save with only add-key dirty adds the key, not commit identity", async () => {
      when("add_trusted_signing_key", undefined);
      const wrapper = mountPage();
      await flushPromises();

      // Open the add-key form and type a key; leave commit identity untouched.
      await wrapper
        .findAll("button")
        .find((b) => b.text().includes("Add a signing public key"))!
        .trigger("click");
      await flushPromises();
      await wrapper.find("textarea").setValue("ssh-ed25519 AAAA key");

      const p = leaveGuard()();
      await flushPromises();

      const modal = wrapper.find('[aria-label="Unsaved changes"]');
      await modal
        .findAll("button")
        .find((b) => b.text().includes("Save and leave"))!
        .trigger("click");

      await expect(p).resolves.toBe(true);
      // The paste form routes through the unified server-detect command (D7),
      // not the SSH-only add_trusted_key.
      expect(invoke).toHaveBeenCalledWith("add_trusted_signing_key", {
        armored: "ssh-ed25519 AAAA key",
        label: "signer",
      });
      expect(invoke).not.toHaveBeenCalledWith(
        "set_commit_identity",
        expect.anything(),
      );
    });
  });

  describe("display-language picker", () => {
    function findLanguagePicker(wrapper: ReturnType<typeof mountPage>) {
      // findAllComponents is typed as DOMWrapper[] in this @vue/test-utils
      // build but really returns VueWrapper[] at runtime — cast through unknown.
      return (
        wrapper.findAllComponents(
          BaseSegmentedControl,
        ) as unknown as VueWrapper<any>[]
      ).find((c) => c.props("name") === "display-language");
    }

    it("applies a pinned locale in-memory first, then persists it", async () => {
      when("get_app_config", { secure_screen: true }); // no locale ⇒ "system"
      const { wrapper, toast } = mountWithApp(SettingsPage);
      await flushPromises();

      const picker = findLanguagePicker(wrapper)!;
      picker.vm.$emit("change", "zh-CN");
      await flushPromises();

      // Apply-then-persist: a persist failure can't leave app.json pinned to a
      // locale the picker already reverted from.
      expect(setLocale).toHaveBeenCalledWith("zh-CN");
      expect(invoke).toHaveBeenCalledWith("set_locale_pref", {
        locale: "zh-CN",
      });
      expect(
        toast.toasts.value.some((t) => t.message.includes("Display language")),
      ).toBe(true);
    });

    it("rolls back to the prior selection when persisting fails", async () => {
      when("get_app_config", { secure_screen: true, locale: "en" }); // prior = en
      reject("set_locale_pref", { code: "CONFIG_ERROR", message: "no" });
      const { wrapper, toast } = mountWithApp(SettingsPage);
      await flushPromises();

      const picker = findLanguagePicker(wrapper)!;
      picker.vm.$emit("change", "zh-CN");
      await flushPromises();

      // Picker reverted to the prior selection; a danger toast explains.
      expect(picker?.props("modelValue")).toBe("en");
      expect(
        toast.toasts.value.some((t) =>
          t.message.includes("Couldn't save display language"),
        ),
      ).toBe(true);
    });

    it("'system' resolves through the backend and clears the override", async () => {
      when("get_app_config", { secure_screen: true, locale: "en" });
      when("resolved_locale", "zh-CN");
      const wrapper = mountPage();
      await flushPromises();

      const picker = findLanguagePicker(wrapper)!;
      picker.vm.$emit("change", "system");
      await flushPromises();

      // "Track system" applies the backend's resolved locale and clears the pin.
      expect(invoke).toHaveBeenCalledWith("resolved_locale");
      expect(setLocale).toHaveBeenCalledWith("zh-CN"); // normalizeSupported passthrough
      expect(invoke).toHaveBeenCalledWith("set_locale_pref", { locale: null });
    });
  });
});
