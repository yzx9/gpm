// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import EntryEditPage from "./EntryEditPage.vue";

vi.mock("@tauri-apps/api/core");

const { mockReplace } = vi.hoisted(() => ({ mockReplace: vi.fn() }));

vi.mock("vue-router", () => ({
  createRouter: vi.fn(),
  createWebHashHistory: vi.fn(),
  useRouter: () => ({ push: vi.fn(), replace: mockReplace, back: vi.fn() }),
  useRoute: () => ({
    params: { pathMatch: "servers/prod" },
    query: {},
    name: "entryEdit",
    path: "/edit/servers/prod",
    fullPath: "/edit/servers/prod",
  }),
}));

describe("EntryEditPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "show_password")
        return Promise.resolve({ password: "s3cret", notes: "note line" });
      if (cmd === "edit_secret")
        return Promise.resolve({ kind: "written", commit: "abc1234" });
      return Promise.resolve(undefined);
    });
  });

  it("fetches the body on mount and prefills the fields", async () => {
    const w = mountWithApp(EntryEditPage).wrapper;
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith("show_password", {
      entryPath: "servers/prod",
    });
    expect(
      (w.find('input[id="e-password"]').element as HTMLInputElement).value,
    ).toBe("s3cret");
  });

  it("Save edits and returns to the read view", async () => {
    const w = mountWithApp(EntryEditPage).wrapper;
    await flushPromises();
    await w.find('input[id="e-password"]').setValue("newpass");
    await w.find("form").trigger("submit");
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith("edit_secret", {
      name: "servers/prod",
      parts: { password: "newpass", attributes: [], body: "note line" },
    });
    expect(mockReplace).toHaveBeenCalledWith({
      name: "entry",
      params: { pathMatch: "servers/prod" },
    });
  });

  it("Back returns to the read view without saving", async () => {
    const w = mountWithApp(EntryEditPage).wrapper;
    await flushPromises();
    await w.find('button[aria-label="Back"]').trigger("click");
    await flushPromises();
    expect(mockReplace).toHaveBeenCalledWith({
      name: "entry",
      params: { pathMatch: "servers/prod" },
    });
    expect(invoke).not.toHaveBeenCalledWith("edit_secret", expect.anything());
  });

  it("blocks editing for a binary attachment (hint, disabled save, no base64 body)", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "show_password")
        return Promise.resolve({
          password: "",
          notes: "",
          has_totp: false,
          attachment: { filename: "x.bin", size: 10 },
        });
      return Promise.resolve(undefined);
    });
    const w = mountWithApp(EntryEditPage).wrapper;
    await flushPromises();

    // loadBody detected the attachment, set the hint, and early-returned.
    expect(w.text()).toContain("Attachments can't be edited yet");
    // canSave is false for an attachment — Save stays disabled.
    expect(
      w.find('button[type="submit"]').attributes("disabled"),
    ).toBeDefined();
    // The base64 body never reached the editor textarea.
    expect((w.find("textarea").element as HTMLTextAreaElement).value).toBe("");
    // And no edit_secret write was attempted.
    expect(invoke).not.toHaveBeenCalledWith("edit_secret", expect.anything());
  });

  it("blocks editing for non-UTF-8 content (hint, disabled save, no write)", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "show_password")
        return Promise.resolve({
          password: "pw",
          notes: "",
          has_totp: false,
          attachment: null,
          edit_blocked: "nonUtf8",
          version: null,
        });
      return Promise.resolve(undefined);
    });
    const w = mountWithApp(EntryEditPage).wrapper;
    await flushPromises();

    // loadBody detected non-UTF-8 content, set the hint, and early-returned.
    expect(w.text()).toContain("non-UTF-8");
    // canSave is false — Save stays disabled.
    expect(
      w.find('button[type="submit"]').attributes("disabled"),
    ).toBeDefined();
    // No edit_secret write was attempted (the lossy view is never saved back).
    expect(invoke).not.toHaveBeenCalledWith("edit_secret", expect.anything());
  });

  it("passes the captured show_password `version` as baseOid on save (R026)", async () => {
    // The edit screen captures the blob oid atomically with the decrypt and
    // sends it back as baseOid so a stale edit surfaces entry_conflict
    // instead of silently clobbering a teammate's change.
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "show_password")
        return Promise.resolve({
          password: "s3cret",
          notes: "note line",
          edit_blocked: null,
          version: "oid-cafe",
        });
      if (cmd === "edit_secret")
        return Promise.resolve({ kind: "written", commit: "abc1234" });
      return Promise.resolve(undefined);
    });
    const w = mountWithApp(EntryEditPage).wrapper;
    await flushPromises();
    await w.find('input[id="e-password"]').setValue("newpass");
    await w.find("form").trigger("submit");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("edit_secret", {
      name: "servers/prod",
      parts: { password: "newpass", attributes: [], body: "note line" },
      baseOid: "oid-cafe",
    });
  });

  it("on entry_conflict outcome, surfaces the EntryConflictModal with the entry name", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "show_password")
        return Promise.resolve({
          password: "s3cret",
          notes: "note line",
          edit_blocked: null,
          version: "oid-1",
        });
      if (cmd === "edit_secret")
        return Promise.resolve({
          kind: "entry_conflict",
          name: "servers/prod",
          base_oid: "oid-1",
          current_oid: "oid-2",
          remote_tip: "tip-3",
          op: "edit",
        });
      return Promise.resolve(undefined);
    });
    const w = mountWithApp(EntryEditPage).wrapper;
    await flushPromises();
    await w.find('input[id="e-password"]').setValue("newpass");
    await w.find("form").trigger("submit");
    await flushPromises();

    // The per-entry conflict modal renders with the entry name (R026).
    expect(w.text()).toContain("This secret changed elsewhere");
    expect(w.text()).toContain("servers/prod");
  });

  it("round-trips an edited attribute value in the saved parts", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "show_password")
        return Promise.resolve({
          password: "s3cret",
          notes: "",
          attributes: [{ key: "user", value: "alice" }],
        });
      if (cmd === "edit_secret")
        return Promise.resolve({ kind: "written", commit: "abc1234" });
      return Promise.resolve(undefined);
    });
    const w = mountWithApp(EntryEditPage).wrapper;
    await flushPromises();

    // Form inputs in order: password, attribute key, attribute value.
    await w.findAll("input")[2].setValue("alice-new");
    await w.find("form").trigger("submit");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("edit_secret", {
      name: "servers/prod",
      parts: {
        password: "s3cret",
        attributes: [{ key: "user", value: "alice-new" }],
        body: "",
      },
    });
  });

  it("an attribute key with ': ' disables Save (Rust SecretInvalid parity)", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "show_password")
        return Promise.resolve({
          password: "s3cret",
          notes: "",
          attributes: [{ key: "bad: key", value: "v" }],
        });
      if (cmd === "edit_secret")
        return Promise.resolve({ kind: "written", commit: "abc1234" });
      return Promise.resolve(undefined);
    });
    const w = mountWithApp(EntryEditPage).wrapper;
    await flushPromises();

    // hasInvalidKey ⇒ Save stays disabled and no write is attempted.
    expect(
      w.find('button[type="submit"]').attributes("disabled"),
    ).toBeDefined();
    await w.find("form").trigger("submit");
    await flushPromises();
    expect(invoke).not.toHaveBeenCalledWith("edit_secret", expect.anything());
  });

  it("renders a warning (not red error) when the store needs an age plugin binary", async () => {
    // edit_secret rejects PLUGIN_UNAVAILABLE: the store has an age plugin
    // recipient whose binary can't run here. The alert must surface as a
    // warning (role=status) carrying the backend message — not a red danger.
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "show_password")
        return Promise.resolve({ password: "s3cret", notes: "note line" });
      if (cmd === "edit_secret")
        return Promise.reject({
          code: "PLUGIN_UNAVAILABLE",
          message:
            "Encryption needs the age plugin 'age-plugin-yubikey', which can't run on Android",
        });
      return Promise.resolve(undefined);
    });
    const w = mountWithApp(EntryEditPage).wrapper;
    await flushPromises();
    await w.find('input[id="e-password"]').setValue("newpass");
    await w.find("form").trigger("submit");
    await flushPromises();

    const alert = w.find("[role='status']");
    expect(alert.exists()).toBe(true);
    expect(alert.text()).toContain("age-plugin-yubikey");
    expect(w.find("[role='alert']").exists()).toBe(false);
  });

  it("renders a red error for a generic save failure (baseline)", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "show_password")
        return Promise.resolve({ password: "s3cret", notes: "note line" });
      if (cmd === "edit_secret")
        return Promise.reject({ code: "DECRYPT_FAILED", message: "boom" });
      return Promise.resolve(undefined);
    });
    const w = mountWithApp(EntryEditPage).wrapper;
    await flushPromises();
    await w.find('input[id="e-password"]').setValue("newpass");
    await w.find("form").trigger("submit");
    await flushPromises();

    expect(w.find("[role='alert']").exists()).toBe(true);
    expect(w.find("[role='status']").exists()).toBe(false);
  });
});
