// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
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

/// The whole point of the backend-held refactor: the generated secret identity
/// must never appear in any IPC payload. Scans every recorded `invoke` call —
/// both for a secret-looking string AND for any command receiving an `identity`
/// field (the structural regression this refactor removed).
function expectNoSecretCrossedIPC() {
  for (const [cmd, args] of vi.mocked(invoke).mock.calls) {
    if (
      args &&
      typeof args === "object" &&
      "identity" in (args as Record<string, unknown>)
    ) {
      throw new Error(
        `${cmd} was invoked with an \`identity\` field — the secret must stay backend-side`,
      );
    }
  }
  const dump = JSON.stringify(vi.mocked(invoke).mock.calls);
  expect(dump).not.toContain("AGE-SECRET-KEY");
  expect(dump).not.toContain("BEGIN OPENSSH PRIVATE KEY");
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

  it("generates an age identity (recipient only) and creates a local-only store", async () => {
    mockInvoke({
      generate_identity: (args) => {
        expect(args).toEqual({ kind: "age", passphrase: null });
        return "age1recipientkey";
      },
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
      complete_setup_from_file: (args) => {
        // No `identity` field — the secret is consumed from backend state.
        expect(args).toEqual({ passphrase: null });
        return undefined;
      },
    });

    const wrapper = mount(CreateFlow);
    await flushPromises();

    // No recipient yet — the panel is absent.
    expect(wrapper.text()).not.toContain("Recipient");

    await clickButton(wrapper, "Generate identity");
    expect(invoke).toHaveBeenCalledWith("generate_identity", {
      kind: "age",
      passphrase: null,
    });
    expect(wrapper.text()).toContain("age1recipientkey");

    await submit(wrapper);

    expect(invoke).toHaveBeenCalledWith("create_store", expect.anything());
    expect(invoke).toHaveBeenCalledWith(
      "complete_setup_from_file",
      expect.anything(),
    );
    // No remote → no first push.
    expect(invoke).not.toHaveBeenCalledWith("push_repo");
    expect(wrapper.emitted("done")).toHaveLength(1);

    // The invariant: the generated secret never crosses IPC.
    expectNoSecretCrossedIPC();
  });

  it("renders the public recipient but never holds a secret", async () => {
    mockInvoke({
      generate_identity: () => "age1pub",
    });
    const wrapper = mount(CreateFlow);
    await flushPromises();
    await clickButton(wrapper, "Generate identity");

    expect(wrapper.text()).toContain("age1pub");
    // No secret identity string exists anywhere in the rendered output.
    expect(wrapper.text()).not.toContain("AGE-SECRET-KEY");
    expectNoSecretCrossedIPC();
  });

  // ── SSH identity path ──────────────────────────────────────────────────

  it("generates an SSH identity (recipient only) and seeds it as the recipient", async () => {
    mockInvoke({
      generate_identity: (args) => {
        expect(args).toEqual({ kind: "ssh", passphrase: null });
        return "ssh-ed25519 AAAApub";
      },
      create_store: (args) => {
        expect(args).toMatchObject({
          recipient: "ssh-ed25519 AAAApub",
          repoUrl: null,
        });
        return undefined;
      },
      complete_setup_from_file: (args) => {
        expect(args).toEqual({ passphrase: null });
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

    expect(wrapper.emitted("done")).toHaveLength(1);
    // The SSH private key never crosses IPC.
    expectNoSecretCrossedIPC();
  });

  // ── remote paths ───────────────────────────────────────────────────────

  it("creates + pushes when an HTTPS remote is given", async () => {
    const calls: string[] = [];
    mockInvoke({
      generate_identity: () => "age1r",
      create_store: () => {
        calls.push("create_store");
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

    const wrapper = mount(CreateFlow);
    await flushPromises();
    await clickButton(wrapper, "Generate identity");
    await wrapper
      .find('input[id="repo-url"]')
      .setValue("https://example.com/r.git");
    await wrapper.find('input[id="pat"]').setValue("my-pat");
    await submit(wrapper);

    // create → complete_setup_from_file → push_repo, in that order (deferred push).
    expect(calls).toEqual([
      "create_store",
      "complete_setup_from_file",
      "push_repo",
    ]);
    expect(invoke).toHaveBeenCalledWith("create_store", {
      recipient: "age1r",
      repoUrl: "https://example.com/r.git",
      pat: "my-pat",
      sshKey: null,
      sshPassphrase: null,
    });
    expect(wrapper.emitted("done")).toHaveLength(1);
    expectNoSecretCrossedIPC();
  });

  it("sends SSH auth for an SSH remote URL", async () => {
    mockInvoke({
      generate_identity: () => "age1r",
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
      complete_setup_from_file: () => undefined,
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
      generate_identity: () => "age1r",
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
      generate_identity: () => "age1r",
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
      generate_identity: () => "age1r",
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
      "complete_setup_from_file",
      expect.anything(),
    );
    expect(wrapper.emitted("done")).toBeUndefined();
  });

  it("surfaces a complete_setup_from_file failure without pushing or emitting done", async () => {
    // The store was created but the identity didn't persist — the orphan-
    // recipient concern. The first push must be skipped (identity not durable)
    // and the user must not be navigated to entries.
    mockInvoke({
      generate_identity: () => "age1r",
      create_store: () => undefined,
      complete_setup_from_file: () => {
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
    expect(invoke).toHaveBeenCalledWith(
      "complete_setup_from_file",
      expect.anything(),
    );
    expect(invoke).not.toHaveBeenCalledWith("push_repo", expect.anything());
    expect(wrapper.emitted("done")).toBeUndefined();
  });

  // ── deferred-push atomicity ────────────────────────────────────────────

  it("blocks navigation when the first push fails (store is created locally)", async () => {
    mockInvoke({
      generate_identity: () => "age1r",
      create_store: () => undefined,
      complete_setup_from_file: () => undefined,
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

    // create + complete_setup_from_file succeeded; the push failed → stay on
    // the page with a visible error so the user knows the store didn't sync.
    expect(invoke).toHaveBeenCalledWith("push_repo");
    expect(wrapper.find("[role='alert']").text()).toContain(
      "remote unreachable",
    );
    expect(wrapper.find("[role='alert']").text()).toContain("saved locally");
    expect(wrapper.emitted("done")).toBeUndefined();
  });

  // ── clearing the staged identity ───────────────────────────────────────

  it("drops the staged identity when switching kind", async () => {
    mockInvoke({ generate_identity: () => "age1r" });
    const wrapper = mount(CreateFlow);
    await flushPromises();
    await clickButton(wrapper, "Generate identity");
    expect(wrapper.text()).toContain("age1r");

    // Switching to SSH must drop the staged age identity so it can't be saved
    // stale — the backend is told to forget it, and the recipient panel clears.
    await clickButton(wrapper, "SSH (ed25519)");
    expect(invoke).toHaveBeenCalledWith("clear_pending_identity");
    expect(wrapper.text()).not.toContain("age1r");
  });

  // ── SSH passphrase is fixed at mint time ───────────────────────────────

  it("locks the SSH passphrase field after generation and reuses the minted value", async () => {
    let completeArgs: Record<string, unknown> | undefined;
    mockInvoke({
      generate_identity: (args) => {
        expect(args).toEqual({ kind: "ssh", passphrase: "original-pass" });
        return "ssh-ed25519 AAAApub";
      },
      is_configured: () => false,
      create_store: () => undefined,
      complete_setup_from_file: (args) => {
        completeArgs = args;
        return undefined;
      },
    });

    const wrapper = mount(CreateFlow);
    await flushPromises();
    await clickButton(wrapper, "SSH (ed25519)");
    const passInput = wrapper.find('input[id="create-passphrase"]');
    // Editable before generation.
    expect(passInput.attributes("disabled")).toBeUndefined();
    await passInput.setValue("original-pass");
    await wrapper
      .find('input[id="create-passphrase-confirm"]')
      .setValue("original-pass");
    await clickButton(wrapper, "Generate SSH key");

    // Locked after generation — for SSH the passphrase is fixed at mint time.
    expect(passInput.attributes("disabled")).toBeDefined();

    // Even if the live field diverges (a mid-generate keystroke before the lock
    // took effect, or a programmatic change), complete must reuse the value that
    // actually minted the key — SSH derives its recipient from the PEM it encrypted.
    await passInput.setValue("changed-pass");
    await submit(wrapper);

    expect(completeArgs).toEqual({ passphrase: "original-pass" });
  });

  it("rejects an SSH generate when the passphrase and confirm do not match", async () => {
    mockInvoke({ generate_identity: () => "ssh-ed25519 AAAApub" });

    const wrapper = mount(CreateFlow);
    await flushPromises();
    await clickButton(wrapper, "SSH (ed25519)");
    await wrapper.find('input[id="create-passphrase"]').setValue("secret");
    await wrapper
      .find('input[id="create-passphrase-confirm"]')
      .setValue("different");
    await clickButton(wrapper, "Generate SSH key");

    // The passphrase is baked into the SSH key at mint time, so a mismatch must
    // block the mint entirely — the key must not be generated with a typo.
    expect(invoke).not.toHaveBeenCalledWith(
      "generate_identity",
      expect.anything(),
    );
    expect(wrapper.text()).toContain("Passphrases do not match");
    expect(wrapper.text()).not.toContain("ssh-ed25519");
  });

  // ── retry after a non-fatal push failure ───────────────────────────────

  it("on re-submit after a push failure, retries only the push (no re-bootstrap)", async () => {
    const seq: string[] = [];
    let configuredCalls = 0;
    let pushCalls = 0;
    mockInvoke({
      generate_identity: () => "age1r",
      is_configured: () => {
        configuredCalls += 1;
        // 1st submit: store not yet configured; 2nd: configured (identity saved).
        return configuredCalls >= 2;
      },
      create_store: () => {
        seq.push("create_store");
        return undefined;
      },
      complete_setup_from_file: () => {
        seq.push("complete_setup_from_file");
        return undefined;
      },
      push_repo: () => {
        pushCalls += 1;
        seq.push("push_repo");
        // First push fails; the retry succeeds.
        if (pushCalls === 1) {
          throw { code: "NETWORK_ERROR", message: "remote unreachable" };
        }
        return undefined;
      },
    });

    const wrapper = mount(CreateFlow);
    await flushPromises();
    await clickButton(wrapper, "Generate identity");
    await wrapper
      .find('input[id="repo-url"]')
      .setValue("https://example.com/r.git");
    await submit(wrapper);

    // 1st submit: full bootstrap, then a failed push. No done emitted.
    expect(seq).toEqual([
      "create_store",
      "complete_setup_from_file",
      "push_repo",
    ]);
    expect(wrapper.find("[role='alert']").text()).toContain(
      "remote unreachable",
    );
    expect(wrapper.emitted("done")).toBeUndefined();

    // Retry: the store is configured now → must NOT re-bootstrap (create_store
    // clears config + rm -rf's the repo, and the staged identity is consumed,
    // so a re-run would strand the store). Only the push is retried.
    await submit(wrapper);

    expect(seq).toEqual([
      "create_store",
      "complete_setup_from_file",
      "push_repo",
      "push_repo",
    ]);
    expect(wrapper.emitted("done")).toHaveLength(1);
  });
});
