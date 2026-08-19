// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import CreateCustomPage from "./CreateCustomPage.vue";

vi.mock("@tauri-apps/api/core");

const { mockReplace } = vi.hoisted(() => ({ mockReplace: vi.fn() }));

vi.mock("vue-router", () => ({
  createRouter: vi.fn(),
  createWebHashHistory: vi.fn(),
  useRouter: () => ({ push: vi.fn(), replace: mockReplace, back: vi.fn() }),
  useRoute: () => ({
    params: {},
    query: {},
    name: "createCustom",
    path: "/",
    fullPath: "/",
  }),
}));

describe("CreateCustomPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "create_secret")
        return Promise.resolve({ kind: "written", commit: "abc1234" });
      if (cmd === "lookup_template") return Promise.resolve(null);
      if (cmd === "preview_create") return Promise.resolve(null);
      return Promise.resolve(undefined);
    });
  });

  it("Save is disabled until both name and content are filled", async () => {
    const w = mountWithApp(CreateCustomPage).wrapper;
    await flushPromises();
    const save = w
      .findAll("button")
      .find((b) => b.text().includes("Save secret"))!;
    expect((save.element as HTMLButtonElement).disabled).toBe(true);
    await w.find('input[id="c-name"]').setValue("misc/foo");
    expect((save.element as HTMLButtonElement).disabled).toBe(true);
    await w.find('textarea[id="c-content"]').setValue("hunter2");
    expect((save.element as HTMLButtonElement).disabled).toBe(false);
  });

  it("Save is disabled with the YAML warning when content has a --- line (A004)", async () => {
    // A `---`-prefixed line marks a legacy gopass YAML secret; gpm shows
    // those read-only, so the create form blocks inline instead of letting
    // the app create an entry it would immediately refuse to edit.
    const w = mountWithApp(CreateCustomPage).wrapper;
    await flushPromises();
    await w.find('input[id="c-name"]').setValue("misc/foo");
    await w.find('textarea[id="c-content"]').setValue("pw\n---\nk: v");
    await flushPromises();

    expect(w.text()).toContain("legacy gopass YAML secret");
    const save = w
      .findAll("button")
      .find((b) => b.text().includes("Save secret"))!;
    expect((save.element as HTMLButtonElement).disabled).toBe(true);
    expect(invoke).not.toHaveBeenCalledWith("create_secret", expect.anything());

    // Removing the marker re-enables the form.
    await w.find('textarea[id="c-content"]').setValue("pw\nk: v");
    await flushPromises();
    expect((save.element as HTMLButtonElement).disabled).toBe(false);
  });

  it("A bare --- document stays blocked; armor and ---password stay storable", async () => {
    // Mirrors `is_yaml_secret_content`: the first line counts only as a bare
    // `---` document (the password line is never a marker); PEM armor
    // (`-----BEGIN`, starts `----`) and a password merely starting `---` are
    // editable AKV content gopass itself round-trips.
    const w = mountWithApp(CreateCustomPage).wrapper;
    await flushPromises();
    await w.find('input[id="c-name"]').setValue("misc/foo");
    const save = w
      .findAll("button")
      .find((b) => b.text().includes("Save secret"))!;
    const content = w.find('textarea[id="c-content"]');

    await content.setValue("---\nk: v");
    await flushPromises();
    expect((save.element as HTMLButtonElement).disabled).toBe(true);

    await content.setValue(
      "pw\n-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----",
    );
    await flushPromises();
    expect((save.element as HTMLButtonElement).disabled).toBe(false);

    await content.setValue("---hunter2\nuser: alice");
    await flushPromises();
    expect((save.element as HTMLButtonElement).disabled).toBe(false);
  });

  it("Save creates the secret and returns to entries", async () => {
    const w = mountWithApp(CreateCustomPage).wrapper;
    await flushPromises();
    await w.find('input[id="c-name"]').setValue("misc/foo");
    await w.find('textarea[id="c-content"]').setValue("hunter2");
    await w.find("form").trigger("submit");
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith("create_secret", {
      name: "misc/foo",
      content: "hunter2",
    });
    expect(mockReplace).toHaveBeenCalledWith({ name: "entries" });
  });

  it("on entry_conflict outcome, surfaces the per-entry modal (create op)", async () => {
    // R026: a teammate created the same name — the create surfaces the
    // EntryConflictModal (create op) instead of overwriting silently. Pins the
    // create-conflict modal render (heading + name + op-specific copy), the one
    // base-version-aware op with no prior frontend coverage.
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "create_secret")
        return Promise.resolve({
          kind: "entry_conflict",
          name: "misc/foo",
          base_oid: "",
          current_oid: "theirs-oid",
          remote_tip: "tip",
          op: "create",
        });
      if (cmd === "lookup_template") return Promise.resolve(null);
      if (cmd === "preview_create") return Promise.resolve(null);
      return Promise.resolve(undefined);
    });
    const w = mountWithApp(CreateCustomPage).wrapper;
    await flushPromises();
    await w.find('input[id="c-name"]').setValue("misc/foo");
    await w.find('textarea[id="c-content"]').setValue("hunter2");
    await w.find("form").trigger("submit");
    await flushPromises();

    // The per-entry conflict sheet shows the create-op heading + the name.
    expect(w.text()).toContain("This name is already in use");
    expect(w.text()).toContain("misc/foo");
    // op-specific step-1 labels render (create keeps existing / overwrites).
    expect(w.text()).toContain("Keep the existing one");
    expect(w.text()).toContain("Overwrite with mine");
  });

  it("Back returns to the pick step", async () => {
    const w = mountWithApp(CreateCustomPage).wrapper;
    await flushPromises();
    await w.find('button[aria-label="Back"]').trigger("click");
    await flushPromises();
    expect(mockReplace).toHaveBeenCalledWith({ name: "create" });
  });

  it("renders a warning when the store needs an age plugin binary", async () => {
    // create_secret rejects PLUGIN_UNAVAILABLE: the store has an age plugin
    // recipient whose binary can't run here. The alert must be a warning
    // (role=status) carrying the backend message, not a red danger (role=alert).
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "create_secret")
        return Promise.reject({
          code: "PLUGIN_UNAVAILABLE",
          message:
            "Encryption needs the age plugin 'age-plugin-yubikey', which can't run on Android",
        });
      if (cmd === "lookup_template") return Promise.resolve(null);
      if (cmd === "preview_create") return Promise.resolve(null);
      return Promise.resolve(undefined);
    });
    const w = mountWithApp(CreateCustomPage).wrapper;
    await flushPromises();
    await w.find('input[id="c-name"]').setValue("misc/foo");
    await w.find('textarea[id="c-content"]').setValue("hunter2");
    await w.find("form").trigger("submit");
    await flushPromises();

    const alert = w.find("[role='status']");
    expect(alert.exists()).toBe(true);
    expect(alert.text()).toContain("age-plugin-yubikey");
    expect(w.find("[role='alert']").exists()).toBe(false);
  });

  it("renders a red error for a generic create failure (baseline)", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "create_secret")
        return Promise.reject({ code: "DECRYPT_FAILED", message: "boom" });
      if (cmd === "lookup_template") return Promise.resolve(null);
      if (cmd === "preview_create") return Promise.resolve(null);
      return Promise.resolve(undefined);
    });
    const w = mountWithApp(CreateCustomPage).wrapper;
    await flushPromises();
    await w.find('input[id="c-name"]').setValue("misc/foo");
    await w.find('textarea[id="c-content"]').setValue("hunter2");
    await w.find("form").trigger("submit");
    await flushPromises();

    expect(w.find("[role='alert']").exists()).toBe(true);
    expect(w.find("[role='status']").exists()).toBe(false);
  });

  // ── Gate re-lock (issue #20): the mask does not unmount the page ────────

  it("a gate re-lock wipes a filled form and marks the drafts notice", async () => {
    const m = mountWithApp(CreateCustomPage);
    await flushPromises();
    await m.wrapper.find('input[id="c-name"]').setValue("misc/foo");
    await m.wrapper.find('textarea[id="c-content"]').setValue("hunter2");

    m.appLock.setAppLocked(true, "idle");
    await flushPromises();

    expect(
      (m.wrapper.find('input[id="c-name"]').element as HTMLInputElement).value,
    ).toBe("");
    expect(
      (
        m.wrapper.find('textarea[id="c-content"]')
          .element as HTMLTextAreaElement
      ).value,
    ).toBe("");
    expect(m.draftsNotice.consume()).toBe(true); // draft lost → toast fires
  });

  it("a gate re-lock on an empty form does not mark the notice", async () => {
    const m = mountWithApp(CreateCustomPage);
    await flushPromises();

    m.appLock.setAppLocked(true, "idle");
    await flushPromises();

    expect(m.draftsNotice.consume()).toBe(false); // nothing was lost
  });
});
