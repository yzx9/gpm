// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import SshKeyFields from "./SshKeyFields.vue";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const KEY_PAIR = {
  private_key:
    "-----BEGIN OPENSSH PRIVATE KEY-----\n GENERATED \n-----END OPENSSH PRIVATE KEY-----",
  public_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIgenerated",
};

const findButton = (w: ReturnType<typeof mount>, text: string) =>
  w.findAll("button").find((b) => b.text().includes(text));

describe("SshKeyFields", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
    vi.mocked(invoke).mockResolvedValue(KEY_PAIR);
  });

  it("paste branch: key + passphrase two-way bind through the models", async () => {
    const w = mount(SshKeyFields, {
      props: { sshKey: "", sshPassphrase: "" },
    });
    await w.get("#ssh-key").setValue("pasted-key");
    await w.get("#ssh-passphrase").setValue("secret");
    expect(w.emitted("update:sshKey")).toEqual([["pasted-key"]]);
    expect(w.emitted("update:sshPassphrase")).toEqual([["secret"]]);
  });

  it("generate branch: generates a keypair and shows the public half", async () => {
    const w = mount(SshKeyFields, {
      props: { sshKey: "", sshPassphrase: "" },
    });
    // Switch to the Generate tab (segmented control radios).
    await w
      .findAll("label")
      .find((l) => l.text().includes("Generate Key"))!
      .find('input[type="radio"]')
      .trigger("change");
    await flushPromises();

    await findButton(w, "Generate SSH Key")!.trigger("click");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("generate_ssh_key", {
      passphrase: null,
    });
    const keyEvents = w.emitted("update:sshKey")!;
    expect(keyEvents[keyEvents.length - 1]).toEqual([KEY_PAIR.private_key]);
    expect(w.text()).toContain(KEY_PAIR.public_key);
  });

  it("generate with an unconfirmed passphrase does not call keygen and surfaces the error", async () => {
    const w = mount(SshKeyFields, {
      props: { sshKey: "", sshPassphrase: "secret" },
    });
    await w
      .findAll("label")
      .find((l) => l.text().includes("Generate Key"))!
      .find('input[type="radio"]')
      .trigger("change");
    await flushPromises();

    await findButton(w, "Generate SSH Key")!.trigger("click");
    await flushPromises();

    expect(invoke).not.toHaveBeenCalled();
    const errorEvents = w.emitted("update:error")!;
    expect(errorEvents[errorEvents.length - 1]![0]).toContain("confirm your passphrase");
  });

  it("a failed keygen surfaces the backend message and re-enables the button", async () => {
    vi.mocked(invoke).mockRejectedValueOnce({
      code: "INTERNAL",
      message: "keygen blew up",
    });
    const w = mount(SshKeyFields, {
      props: { sshKey: "", sshPassphrase: "" },
    });
    await w
      .findAll("label")
      .find((l) => l.text().includes("Generate Key"))!
      .find('input[type="radio"]')
      .trigger("change");
    await flushPromises();

    await findButton(w, "Generate SSH Key")!.trigger("click");
    await flushPromises();

    const errorEvents = w.emitted("update:error")!;
    expect(errorEvents[errorEvents.length - 1]![0]).toContain("keygen blew up");
    // The spinner reset: the button is interactive again.
    expect(findButton(w, "Generate SSH Key")!.attributes("disabled")).toBeUndefined();
  });

  it("generate passes the passphrase through verbatim (write-path parity)", async () => {
    const w = mount(SshKeyFields, {
      // Both fields carry the same leading/trailing whitespace — the confirm
      // check passes on the raw values, and the keygen must see the same raw
      // passphrase every other write path validates and persists.
      props: { sshKey: "", sshPassphrase: " pw " },
    });
    await w
      .findAll("label")
      .find((l) => l.text().includes("Generate Key"))!
      .find('input[type="radio"]')
      .trigger("change");
    await flushPromises();
    await w.get("#ssh-gen-passphrase-confirm").setValue(" pw ");
    await findButton(w, "Generate SSH Key")!.trigger("click");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("generate_ssh_key", {
      passphrase: " pw ",
    });
  });

  it("reset() clears the generated public key and returns to the paste tab", async () => {
    const w = mount(SshKeyFields, {
      props: { sshKey: "", sshPassphrase: "" },
    });
    await w
      .findAll("label")
      .find((l) => l.text().includes("Generate Key"))!
      .find('input[type="radio"]')
      .trigger("change");
    await flushPromises();
    await findButton(w, "Generate SSH Key")!.trigger("click");
    await flushPromises();
    expect(w.text()).toContain(KEY_PAIR.public_key);

    w.vm.reset();
    await flushPromises();
    // Back on the paste tab: the textarea is visible, the public key is gone,
    // and the models were not touched (they stay owned by the parent).
    expect(w.find("#ssh-key").exists()).toBe(true);
    expect(w.text()).not.toContain(KEY_PAIR.public_key);
    expect(w.emitted("update:sshKey")!.length).toBe(1);
  });

  it("disabled disables the paste inputs", () => {
    const w = mount(SshKeyFields, {
      props: { sshKey: "", sshPassphrase: "", disabled: true },
    });
    expect(w.get("#ssh-key").attributes("disabled")).toBeDefined();
    expect(w.get("#ssh-passphrase").attributes("disabled")).toBeDefined();
  });
});
