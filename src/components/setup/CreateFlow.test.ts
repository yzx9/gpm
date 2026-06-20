// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import { invoke } from "@tauri-apps/api/core";
import CreateFlow from "./CreateFlow.vue";

vi.mock("@tauri-apps/api/core");

/// Branch `invoke` by command name. `done`-emission + call ordering are the
/// things under test, so a per-command map is more robust than a value queue.
function mockInvoke(
  handlers: Record<string, (args?: Record<string, unknown>) => unknown>,
) {
  vi.mocked(invoke).mockImplementation(async (cmd: string, args?: unknown) => {
    const h = handlers[cmd];
    if (h) return h(args as Record<string, unknown> | undefined);
    return undefined;
  });
}

describe("CreateFlow", () => {
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

  // ── age identity path ──────────────────────────────────────────────────

  it("generates an age identity and creates a local-only store", async () => {
    mockInvoke({
      generate_age_identity: () => ({
        identity: "AGE-SECRET-KEY-1ABCDEF",
        recipient: "age1recipientkey",
      }),
      create_store: (args) => {
        expect(args).toEqual({
          recipient: "age1recipientkey",
          repoUrl: null,
          pat: null,
          sshKey: null,
          sshPassphrase: null,
        });
        return undefined;
      },
      complete_setup: (args) => {
        // The generated identity crosses IPC here; never rendered in the UI.
        expect(args).toEqual({
          identity: "AGE-SECRET-KEY-1ABCDEF",
          passphrase: null,
        });
        return undefined;
      },
    });

    const wrapper = mount(CreateFlow);
    await flushPromises();

    // No identity yet — the recipient panel is absent.
    expect(wrapper.text()).not.toContain("Recipient");

    await clickButton(wrapper, "Generate identity");
    expect(invoke).toHaveBeenCalledWith("generate_age_identity");
    expect(wrapper.text()).toContain("age1recipientkey");

    await submit(wrapper);

    expect(invoke).toHaveBeenCalledWith("create_store", expect.anything());
    expect(invoke).toHaveBeenCalledWith("complete_setup", expect.anything());
    // No remote → no first push.
    expect(invoke).not.toHaveBeenCalledWith("push_repo");
    expect(wrapper.emitted("done")).toHaveLength(1);
  });

  it("does not render the secret identity, only the public recipient", async () => {
    mockInvoke({
      generate_age_identity: () => ({
        identity: "AGE-SECRET-KEY-1NEVERRENDERED",
        recipient: "age1pub",
      }),
    });
    const wrapper = mount(CreateFlow);
    await flushPromises();
    await clickButton(wrapper, "Generate identity");

    expect(wrapper.text()).not.toContain("AGE-SECRET-KEY-1NEVERRENDERED");
  });

  // ── SSH identity path ──────────────────────────────────────────────────

  it("generates an SSH keypair and uses the public key as recipient", async () => {
    mockInvoke({
      generate_ssh_key: () => ({
        public_key: "ssh-ed25519 AAAApub",
        private_key: "-----BEGIN OPENSSH PRIVATE KEY-----\npriv\n-----END-----",
      }),
      create_store: (args) => {
        expect(args).toMatchObject({
          recipient: "ssh-ed25519 AAAApub",
          repoUrl: null,
        });
        return undefined;
      },
      complete_setup: (args) => {
        expect(args).toEqual({
          identity: "-----BEGIN OPENSSH PRIVATE KEY-----\npriv\n-----END-----",
          passphrase: null,
        });
        return undefined;
      },
    });

    const wrapper = mount(CreateFlow);
    await flushPromises();

    await clickButton(wrapper, "SSH (ed25519)");
    // Switching kind cleared any prior identity — no recipient yet.
    expect(wrapper.text()).not.toContain("ssh-ed25519");
    await clickButton(wrapper, "Generate SSH key");
    expect(wrapper.text()).toContain("ssh-ed25519 AAAApub");

    await submit(wrapper);

    expect(invoke).toHaveBeenCalledWith("generate_ssh_key", {
      passphrase: null,
    });
    expect(wrapper.emitted("done")).toHaveLength(1);
  });

  // ── remote paths ───────────────────────────────────────────────────────

  it("creates + pushes when an HTTPS remote is given", async () => {
    const calls: string[] = [];
    mockInvoke({
      generate_age_identity: () => ({ identity: "sk", recipient: "age1r" }),
      create_store: () => {
        calls.push("create_store");
        return undefined;
      },
      complete_setup: () => {
        calls.push("complete_setup");
        return undefined;
      },
      push_repo: () => {
        calls.push("push_repo");
        return undefined;
      },
    });

    const wrapper = mount(CreateFlow);
    await flushPromises();
    await clickButton(wrapper, "Generate identity");
    await wrapper
      .find('input[id="repo-url"]')
      .setValue("https://example.com/r.git");
    await wrapper.find('input[id="pat"]').setValue("my-pat");
    await submit(wrapper);

    // create → complete_setup → push_repo, in that order (deferred push).
    expect(calls).toEqual(["create_store", "complete_setup", "push_repo"]);
    expect(invoke).toHaveBeenCalledWith("create_store", {
      recipient: "age1r",
      repoUrl: "https://example.com/r.git",
      pat: "my-pat",
      sshKey: null,
      sshPassphrase: null,
    });
    expect(wrapper.emitted("done")).toHaveLength(1);
  });

  it("sends SSH auth for an SSH remote URL", async () => {
    mockInvoke({
      generate_age_identity: () => ({ identity: "sk", recipient: "age1r" }),
      create_store: (args) => {
        expect(args).toEqual({
          recipient: "age1r",
          repoUrl: "git@github.com:user/repo.git",
          pat: null,
          sshKey: "push-auth-key",
          sshPassphrase: null,
        });
        return undefined;
      },
      complete_setup: () => undefined,
      push_repo: () => undefined,
    });

    const wrapper = mount(CreateFlow);
    await flushPromises();
    await clickButton(wrapper, "Generate identity");
    await wrapper
      .find('input[id="repo-url"]')
      .setValue("git@github.com:user/repo.git");
    await wrapper.find('textarea[id="ssh-key"]').setValue("push-auth-key");
    await submit(wrapper);

    expect(invoke).toHaveBeenCalledWith("push_repo");
    expect(wrapper.emitted("done")).toHaveLength(1);
  });

  // ── validation ─────────────────────────────────────────────────────────

  it("requires an identity before creating", async () => {
    const wrapper = mount(CreateFlow);
    await flushPromises();
    await submit(wrapper);

    expect(wrapper.find("[role='alert']").text()).toBe(
      "Generate an identity first",
    );
    expect(invoke).not.toHaveBeenCalledWith("create_store", expect.anything());
  });

  it("rejects authentication fields without a repository URL", async () => {
    mockInvoke({
      generate_age_identity: () => ({ identity: "sk", recipient: "age1r" }),
    });
    const wrapper = mount(CreateFlow);
    await flushPromises();
    await clickButton(wrapper, "Generate identity");
    // PAT entered but no URL.
    await wrapper.find('input[id="pat"]').setValue("stray-pat");
    await submit(wrapper);

    expect(wrapper.find("[role='alert']").text()).toContain(
      "Enter a repository URL",
    );
    expect(invoke).not.toHaveBeenCalledWith("create_store", expect.anything());
    expect(wrapper.emitted("done")).toBeUndefined();
  });

  it("rejects an SSH remote URL without a push-auth key", async () => {
    mockInvoke({
      generate_age_identity: () => ({ identity: "sk", recipient: "age1r" }),
    });
    const wrapper = mount(CreateFlow);
    await flushPromises();
    await clickButton(wrapper, "Generate identity");
    await wrapper
      .find('input[id="repo-url"]')
      .setValue("git@github.com:user/repo.git");
    await submit(wrapper);

    expect(wrapper.find("[role='alert']").text()).toBe(
      "SSH private key is required for SSH remote URLs",
    );
  });

  it("surfaces a create_store failure without emitting done", async () => {
    mockInvoke({
      generate_age_identity: () => ({ identity: "sk", recipient: "age1r" }),
      create_store: () => {
        throw { code: "STORE_ERROR", message: "disk full" };
      },
    });
    const wrapper = mount(CreateFlow);
    await flushPromises();
    await clickButton(wrapper, "Generate identity");
    await submit(wrapper);

    expect(wrapper.find("[role='alert']").text()).toBe("disk full");
    expect(invoke).not.toHaveBeenCalledWith(
      "complete_setup",
      expect.anything(),
    );
    expect(wrapper.emitted("done")).toBeUndefined();
  });

  it("surfaces a complete_setup failure without pushing or emitting done", async () => {
    // The store was created but the identity didn't persist — the frontend half
    // of the orphan-recipient concern. The first push must be skipped (identity
    // not durable) and the user must not be navigated to entries.
    mockInvoke({
      generate_age_identity: () => ({ identity: "sk", recipient: "age1r" }),
      create_store: () => undefined,
      complete_setup: () => {
        throw { code: "STORE_ERROR", message: "identity save failed" };
      },
    });
    const wrapper = mount(CreateFlow);
    await flushPromises();
    await clickButton(wrapper, "Generate identity");
    await wrapper
      .find('input[id="repo-url"]')
      .setValue("https://example.com/r.git");
    await submit(wrapper);

    expect(wrapper.find("[role='alert']").text()).toBe("identity save failed");
    expect(invoke).toHaveBeenCalledWith("complete_setup", expect.anything());
    expect(invoke).not.toHaveBeenCalledWith("push_repo", expect.anything());
    expect(wrapper.emitted("done")).toBeUndefined();
  });

  // ── deferred-push atomicity ────────────────────────────────────────────

  it("blocks navigation when the first push fails (store is created locally)", async () => {
    mockInvoke({
      generate_age_identity: () => ({ identity: "sk", recipient: "age1r" }),
      create_store: () => undefined,
      complete_setup: () => undefined,
      push_repo: () => {
        throw { code: "NETWORK_ERROR", message: "remote unreachable" };
      },
    });

    const wrapper = mount(CreateFlow);
    await flushPromises();
    await clickButton(wrapper, "Generate identity");
    await wrapper
      .find('input[id="repo-url"]')
      .setValue("https://example.com/r.git");
    await submit(wrapper);

    // create + complete_setup succeeded; the push failed → stay on the page
    // with a visible error so the user knows the store didn't sync.
    expect(invoke).toHaveBeenCalledWith("push_repo");
    expect(wrapper.find("[role='alert']").text()).toContain(
      "remote unreachable",
    );
    expect(wrapper.find("[role='alert']").text()).toContain("saved locally");
    expect(wrapper.emitted("done")).toBeUndefined();
  });
});
