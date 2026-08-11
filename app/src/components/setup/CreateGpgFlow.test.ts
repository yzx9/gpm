// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import CreateGpgFlow from "./CreateGpgFlow.vue";

vi.mock("@tauri-apps/api/core");

/// Branch `invoke` by command name. Call ordering + payloads are the things
/// under test, so a per-command map is more robust than a value queue.
function mockInvoke(
  handlers: Record<string, (args?: Record<string, unknown>) => unknown>,
) {
  vi.mocked(invoke).mockImplementation(async (cmd: string, args?: unknown) => {
    const h = handlers[cmd];
    if (h) return h(args as Record<string, unknown> | undefined);
    // The setup flow resolves the active repo id (for the first push) from the
    // persisted registry; default to a registered repo so resolveActiveRepoId
    // succeeds without each push test restating it.
    if (cmd === "get_app_config")
      return { repositories: ["test-repo"], last_active: "test-repo" };
    return undefined;
  });
}

/// The GPG secret-key armor must never appear in any IPC payload — only its
/// public metadata (uid/fingerprint/recipient) and the S2K passphrase cross.
/// Scans every recorded `invoke` call for the armor block markers.
function expectNoKeyArmorCrossedIPC() {
  const dump = JSON.stringify(vi.mocked(invoke).mock.calls);
  expect(dump).not.toContain("BEGIN PGP PRIVATE KEY BLOCK");
}

describe("CreateGpgFlow", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  async function clickButton(wrapper: ReturnType<typeof mount>, text: string) {
    const btn = wrapper.findAll("button").find((b) => b.text().includes(text));
    if (!btn) throw new Error(`button "${text}" not found`);
    await btn.trigger("click");
    await flushPromises();
  }

  async function submit(wrapper: ReturnType<typeof mount>) {
    await wrapper.find("form").trigger("submit.prevent");
    await flushPromises();
  }

  /// The GPG pick returns public metadata only (the armor stays backend-side).
  function pickGpgKey() {
    return mockInvoke({
      pick_identity_file: () => ({
        key_type: "gpg",
        encrypted: true,
        filename: "secret.asc",
        recipient: "0xCAFEBABE12345678",
        user_id: "Jordan <jordan@example.com>",
        fingerprint: "ABCD0123456789ABCDEF0123456789ABCDEF0123",
        is_recipient: null, // fresh create — no store to match against yet
      }),
    });
  }

  /// Drive the import + S2K verify steps so Create is enabled.
  async function importAndVerify(wrapper: ReturnType<typeof mount>) {
    await clickButton(wrapper, "Import GPG key");
    expect(invoke).toHaveBeenCalledWith("pick_identity_file");
    // Public metadata is shown; the armor is not.
    expect(wrapper.text()).toContain("jordan@example.com");
    expect(wrapper.text()).toContain("ABCD0123");

    await wrapper.find("#gpg-verify-passphrase").setValue("s2k-pass");
    await clickButton(wrapper, "Verify");
    expect(invoke).toHaveBeenCalledWith("verify_picked_identity", {
      passphrase: "s2k-pass",
    });
  }

  // ── local-only happy path ──────────────────────────────────────────────

  it("imports a GPG key, verifies, and creates a local-only store", async () => {
    mockInvoke({
      pick_identity_file: () => ({
        key_type: "gpg",
        encrypted: true,
        filename: "secret.asc",
        recipient: "0xCAFEBABE12345678",
        user_id: "Jordan <jordan@example.com>",
        fingerprint: "ABCD0123456789ABCDEF0123456789ABCDEF0123",
        is_recipient: null,
      }),
      verify_picked_identity: () => ({ recipient: "0xCAFEBABE12345678" }),
      is_configured: () => false,
      create_gpg_store: (args) => {
        // No `identity` field — the secret is consumed from backend state; only
        // the optional remote/auth cross. No remote here.
        expect(args).toEqual({
          repoUrl: null,
          pat: null,
          sshKey: null,
          sshPassphrase: null,
        });
        return undefined;
      },
      complete_setup_from_file: (args) => {
        // GPG: no seal passphrase (the S2K armor is stored byte-unchanged).
        expect(args).toEqual({ passphrase: null });
        return undefined;
      },
    });

    const wrapper = mount(CreateGpgFlow);
    await flushPromises();
    await importAndVerify(wrapper);

    await submit(wrapper);

    expect(invoke).toHaveBeenCalledWith("create_gpg_store", expect.anything());
    expect(invoke).toHaveBeenCalledWith(
      "complete_setup_from_file",
      expect.anything(),
    );
    // No remote → no first push.
    expect(invoke).not.toHaveBeenCalledWith("push_repo");
    expect(wrapper.emitted("done")).toHaveLength(1);

    // The GPG secret-key armor never crosses IPC.
    expectNoKeyArmorCrossedIPC();
  });

  // ── remote path ────────────────────────────────────────────────────────

  it("creates + pushes when an HTTPS remote is given (deferred push)", async () => {
    const calls: string[] = [];
    mockInvoke({
      pick_identity_file: () => ({
        key_type: "gpg",
        encrypted: true,
        filename: "secret.asc",
        recipient: "0xCAFEBABE12345678",
        user_id: "Jordan <jordan@example.com>",
        fingerprint: "ABCD0123456789ABCDEF0123456789ABCDEF0123",
        is_recipient: null,
      }),
      verify_picked_identity: () => ({ recipient: "0xCAFEBABE12345678" }),
      is_configured: () => false,
      create_gpg_store: (args) => {
        expect(args).toEqual({
          repoUrl: "https://example.com/r.git",
          pat: "my-pat",
          sshKey: null,
          sshPassphrase: null,
        });
        calls.push("create_gpg_store");
        return undefined;
      },
      complete_setup_from_file: () => {
        calls.push("complete_setup_from_file");
        return undefined;
      },
      push_repo: () => {
        calls.push("push_repo");
        return undefined;
      },
    });

    const wrapper = mount(CreateGpgFlow);
    await flushPromises();
    await importAndVerify(wrapper);
    await wrapper
      .find('input[id="repo-url"]')
      .setValue("https://example.com/r.git");
    await wrapper.find('input[id="pat"]').setValue("my-pat");
    await submit(wrapper);

    // create → complete_setup_from_file → push_repo, in that order.
    expect(calls).toEqual([
      "create_gpg_store",
      "complete_setup_from_file",
      "push_repo",
    ]);
    expect(wrapper.emitted("done")).toHaveLength(1);
    expectNoKeyArmorCrossedIPC();
  });

  // ── validation ─────────────────────────────────────────────────────────

  it("requires a GPG key before creating", async () => {
    const wrapper = mount(CreateGpgFlow);
    await flushPromises();
    await submit(wrapper);

    expect(wrapper.find("[role='alert']").text()).toBe(
      "Import a GPG key first",
    );
    expect(invoke).not.toHaveBeenCalledWith(
      "create_gpg_store",
      expect.anything(),
    );
  });

  it("requires the S2K passphrase to be verified before creating", async () => {
    pickGpgKey();
    const wrapper = mount(CreateGpgFlow);
    await flushPromises();
    await clickButton(wrapper, "Import GPG key");
    // Picked but not verified.
    await submit(wrapper);

    expect(wrapper.find("[role='alert']").text()).toBe(
      "Verify your GPG key's passphrase first",
    );
    expect(invoke).not.toHaveBeenCalledWith(
      "create_gpg_store",
      expect.anything(),
    );
    expect(wrapper.emitted("done")).toBeUndefined();
  });

  // ── error surfaces ─────────────────────────────────────────────────────

  it("surfaces a create_gpg_store failure without emitting done", async () => {
    mockInvoke({
      pick_identity_file: () => ({
        key_type: "gpg",
        encrypted: true,
        filename: "secret.asc",
        recipient: "0xCAFEBABE12345678",
        user_id: "Jordan <jordan@example.com>",
        fingerprint: "ABCD0123456789ABCDEF0123456789ABCDEF0123",
        is_recipient: null,
      }),
      verify_picked_identity: () => ({ recipient: "0xCAFEBABE12345678" }),
      create_gpg_store: () => {
        throw { code: "STORE_ERROR", message: "disk full" };
      },
    });
    const wrapper = mount(CreateGpgFlow);
    await flushPromises();
    await importAndVerify(wrapper);
    await submit(wrapper);

    expect(wrapper.find("[role='alert']").text()).toBe("disk full");
    expect(invoke).not.toHaveBeenCalledWith(
      "complete_setup_from_file",
      expect.anything(),
    );
    expect(wrapper.emitted("done")).toBeUndefined();
  });

  it("blocks navigation when the first push fails (store is created locally)", async () => {
    mockInvoke({
      pick_identity_file: () => ({
        key_type: "gpg",
        encrypted: true,
        filename: "secret.asc",
        recipient: "0xCAFEBABE12345678",
        user_id: "Jordan <jordan@example.com>",
        fingerprint: "ABCD0123456789ABCDEF0123456789ABCDEF0123",
        is_recipient: null,
      }),
      verify_picked_identity: () => ({ recipient: "0xCAFEBABE12345678" }),
      is_configured: () => false,
      create_gpg_store: () => undefined,
      complete_setup_from_file: () => undefined,
      push_repo: () => {
        throw { code: "NETWORK_ERROR", message: "remote unreachable" };
      },
    });

    const wrapper = mount(CreateGpgFlow);
    await flushPromises();
    await importAndVerify(wrapper);
    await wrapper
      .find('input[id="repo-url"]')
      .setValue("https://example.com/r.git");
    await submit(wrapper);

    expect(invoke).toHaveBeenCalledWith("push_repo", {
      repoId: "test-repo",
    });
    expect(wrapper.find("[role='alert']").text()).toContain(
      "remote unreachable",
    );
    expect(wrapper.find("[role='alert']").text()).toContain("saved locally");
    expect(wrapper.emitted("done")).toBeUndefined();
  });

  // ── clearing the staged identity ───────────────────────────────────────

  it("drops the staged identity when the picked key is removed", async () => {
    pickGpgKey();
    const wrapper = mount(CreateGpgFlow);
    await flushPromises();
    await clickButton(wrapper, "Import GPG key");
    expect(wrapper.text()).toContain("jordan@example.com");

    await clickButton(wrapper, "Remove");
    expect(invoke).toHaveBeenCalledWith("clear_pending_identity");
    expect(wrapper.text()).not.toContain("jordan@example.com");
  });
});
