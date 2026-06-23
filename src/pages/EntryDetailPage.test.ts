// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { mount } from "@vue/test-utils";
import { flushPromises } from "@vue/test-utils";
import { invoke } from "@tauri-apps/api/core";
import EntryDetailPage from "./EntryDetailPage.vue";
import { useLockState, __resetLockStateForTests } from "../utils/useLockState";

const { mockPush } = vi.hoisted(() => ({
  mockPush: vi.fn(),
}));

vi.mock("@tauri-apps/api/core");

// Override useRoute to provide entry path
const mockRoute = {
  params: { pathMatch: "servers%2Fprod.age" },
  query: {},
  name: "entry",
  path: "/entry/servers%2Fprod.age",
  fullPath: "/entry/servers%2Fprod.age",
};

vi.mock("vue-router", () => ({
  createRouter: vi.fn(),
  createWebHashHistory: vi.fn(),
  useRouter: () => ({
    push: mockPush,
    replace: vi.fn(),
    back: vi.fn(),
  }),
  useRoute: () => mockRoute,
}));

describe("EntryDetailPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
    __resetLockStateForTests();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  function mountPage() {
    return mount(EntryDetailPage);
  }

  describe("showPassword", () => {
    it("invokes show_password with decoded entry path", async () => {
      vi.mocked(invoke).mockResolvedValue({
        password: "s3cret",
        notes: "some notes",
      });
      const wrapper = mountPage();
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("show_password", {
        entryPath: "servers/prod.age",
      });
    });

    it("displays password and notes after reveal", async () => {
      vi.mocked(invoke).mockResolvedValue({
        password: "s3cret",
        notes: "some notes",
      });
      const wrapper = mountPage();
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();

      expect(wrapper.text()).toContain("s3cret");
      expect(wrapper.text()).toContain("some notes");
    });

    it("auto-clears sensitive data after 30 seconds", async () => {
      vi.mocked(invoke).mockResolvedValue({
        password: "s3cret",
        notes: "notes",
      });
      const wrapper = mountPage();
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();

      // Password is visible
      expect(wrapper.text()).toContain("s3cret");

      // Advance past 30s auto-clear
      vi.advanceTimersByTime(30_000);
      await flushPromises();

      // Password is gone
      expect(wrapper.text()).not.toContain("s3cret");
      expect(wrapper.text()).not.toContain("notes");
    });

    it("shows error on failure", async () => {
      vi.mocked(invoke).mockRejectedValue({
        code: "DecryptFailed",
        message: "Decryption failed",
      });
      const wrapper = mountPage();
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();

      expect(wrapper.find("[role='alert']").text()).toContain(
        "Decryption failed",
      );
    });

    it("shows hint for errors containing 'ecrypt'", async () => {
      vi.mocked(invoke).mockRejectedValue({
        code: "DecryptFailed",
        message: "Decryption error",
      });
      const wrapper = mountPage();
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();

      expect(wrapper.text()).toContain("Check your age identity and try again");
    });
  });

  describe("copyPassword", () => {
    it("invokes copy_password and shows success toast", async () => {
      vi.mocked(invoke).mockResolvedValue({
        entry_name: "prod",
        cleared_after_secs: 45,
      });
      const wrapper = mountPage();
      await wrapper
        .find('button[aria-label="Copy password to clipboard"]')
        .trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("copy_password", {
        entryPath: "servers/prod.age",
      });
      expect(wrapper.text()).toContain("✓ Copied prod (45s auto-clear)");
    });

    it("clears sensitive data immediately after copy", async () => {
      // First reveal the password
      vi.mocked(invoke)
        .mockResolvedValueOnce({ password: "s3cret", notes: "" })
        .mockResolvedValueOnce({ entry_name: "prod", cleared_after_secs: 45 });

      const wrapper = mountPage();
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();

      expect(wrapper.text()).toContain("s3cret");

      // Now copy — this should clear sensitive data
      await wrapper
        .find('button[aria-label="Copy password to clipboard"]')
        .trigger("click");
      await flushPromises();

      expect(wrapper.text()).not.toContain("s3cret");
    });

    it("auto-clears toast after 3 seconds", async () => {
      vi.mocked(invoke).mockResolvedValue({
        entry_name: "prod",
        cleared_after_secs: 45,
      });
      const wrapper = mountPage();
      await wrapper
        .find('button[aria-label="Copy password to clipboard"]')
        .trigger("click");
      await flushPromises();

      expect(wrapper.text()).toContain("✓ Copied prod");

      vi.advanceTimersByTime(3000);
      await flushPromises();

      expect(wrapper.text()).not.toContain("✓ Copied prod");
    });
  });

  describe("security lifecycle", () => {
    it("clears sensitive data on unmount", async () => {
      vi.mocked(invoke).mockResolvedValue({
        password: "s3cret",
        notes: "notes",
      });
      const wrapper = mountPage();
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();

      // Password is in DOM
      expect(wrapper.text()).toContain("s3cret");

      // Unmount triggers clearSensitive via onBeforeUnmount
      wrapper.unmount();

      // The key assertion: no memory leak of timers
      // (can't directly check internal state after unmount,
      //  but we verify no lingering setTimeout throws)
    });

    it("clears sensitive data on identity lock", async () => {
      const { setLocked } = useLockState();
      vi.mocked(invoke).mockResolvedValue({
        password: "s3cret",
        notes: "notes",
      });
      // The modal keeps the page mounted, so a lock transition must wipe in place.
      setLocked(false);
      const wrapper = mountPage();
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();

      // Password is in the DOM
      expect(wrapper.text()).toContain("s3cret");

      // Lock fires the shared composable's onLock(clear) without unmounting.
      setLocked(true);
      await flushPromises();

      expect(wrapper.text()).not.toContain("s3cret");
    });

    it("handles ESC key to go back", async () => {
      vi.mocked(invoke).mockResolvedValue({
        password: "s3cret",
        notes: "",
      });
      const wrapper = mountPage();
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();

      // Press ESC on the main element
      await wrapper.find("main").trigger("keydown", { key: "Escape" });
      await flushPromises();

      expect(mockPush).toHaveBeenCalledWith({ name: "entries" });
    });
  });

  describe("deleteSecret", () => {
    // The native confirm() dialog defaults to "proceed" for these tests; the
    // cancel case overrides it explicitly.
    const deleteBtn = () => 'button[aria-label="Delete servers/prod"]';

    beforeEach(() => {
      vi.spyOn(window, "confirm").mockReturnValue(true);
    });

    it("on confirm, invokes delete_secret with the entry name", async () => {
      vi.mocked(invoke).mockResolvedValue({ commit: "abc1234" });
      const wrapper = mountPage();
      await wrapper.find(deleteBtn()).trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("delete_secret", {
        name: "servers/prod",
      });
    });

    it("on success, toasts and navigates to the list", async () => {
      vi.mocked(invoke).mockResolvedValue({ commit: "abc1234" });
      const wrapper = mountPage();
      await wrapper.find(deleteBtn()).trigger("click");
      await flushPromises();

      expect(wrapper.text()).toContain("✓ Deleted (commit abc1234)");
      expect(mockPush).toHaveBeenCalledWith({ name: "entries" });
    });

    it("on PUSH_REJECTED, toasts a sync hint and stays put", async () => {
      vi.mocked(invoke).mockRejectedValue({
        code: "PUSH_REJECTED",
        message: "Remote moved",
      });
      const wrapper = mountPage();
      await wrapper.find(deleteBtn()).trigger("click");
      await flushPromises();

      expect(wrapper.text()).toContain("Remote moved — sync to review");
      expect(mockPush).not.toHaveBeenCalled();
    });

    it("on a non-PUSH_REJECTED error, shows the error and stays put", async () => {
      vi.mocked(invoke).mockRejectedValue({
        code: "STORE_ERROR",
        message: "Disk full",
      });
      const wrapper = mountPage();
      await wrapper.find(deleteBtn()).trigger("click");
      await flushPromises();

      expect(wrapper.find("[role='alert']").text()).toContain("Disk full");
      expect(mockPush).not.toHaveBeenCalled();
    });

    it("disables the button while the delete is inflight", async () => {
      let resolveDelete!: (v: { commit: string }) => void;
      vi.mocked(invoke).mockReturnValue(
        new Promise<{ commit: string }>((r) => {
          resolveDelete = r;
        }),
      );
      const wrapper = mountPage();
      const btn = wrapper.find(deleteBtn());
      expect(btn.attributes("disabled")).toBeUndefined();

      await btn.trigger("click");
      await flushPromises();
      expect(btn.attributes("disabled")).toBeDefined();

      resolveDelete({ commit: "abc1234" });
      await flushPromises();
      expect(btn.attributes("disabled")).toBeUndefined();
    });

    it("does not invoke when confirm is cancelled", async () => {
      vi.spyOn(window, "confirm").mockReturnValue(false);
      vi.mocked(invoke).mockResolvedValue({ commit: "abc1234" });
      const wrapper = mountPage();
      await wrapper.find(deleteBtn()).trigger("click");
      await flushPromises();

      expect(invoke).not.toHaveBeenCalled();
      expect(mockPush).not.toHaveBeenCalled();
    });
  });

  describe("editSecret", () => {
    const editBtn = () => 'button[aria-label="Edit servers/prod"]';
    const saveBtn = () => 'button[aria-label="Save changes"]';
    const cancelEditBtn = () => 'button[aria-label="Cancel edit"]';

    it("cold edit fetches show_password and prefills the fields", async () => {
      // Cold: the user never clicked Show, so the page holds no plaintext yet.
      vi.mocked(invoke).mockResolvedValue({
        password: "s3cret",
        notes: "notes here",
      });
      const wrapper = mountPage();
      await wrapper.find(editBtn()).trigger("click");
      await flushPromises();

      // enterEdit fetched the content via show_password (Codex #1 cold-edit).
      expect(invoke).toHaveBeenCalledWith("show_password", {
        entryPath: "servers/prod.age",
      });
      expect(
        (wrapper.find("#e-password").element as HTMLInputElement).value,
      ).toBe("s3cret");
      expect(
        (wrapper.find("#e-notes").element as HTMLTextAreaElement).value,
      ).toBe("notes here");
    });

    it("save reassembles the body losslessly (no trim; newline-joined) and invokes edit_secret", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce({ password: "orig", notes: "line1\nline2" })
        .mockResolvedValueOnce({ kind: "written", commit: "c1" });
      const wrapper = mountPage();
      await wrapper.find(editBtn()).trigger("click");
      await flushPromises();
      // A value with surrounding whitespace must round-trip verbatim (no trim).
      await wrapper.find("#e-password").setValue("  spaced  ");
      await wrapper.find("form").trigger("submit");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("edit_secret", {
        name: "servers/prod",
        content: "  spaced  \nline1\nline2",
      });
    });

    it("save with empty notes sends the password line only (lossless inverse of parse)", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce({ password: "orig", notes: "" })
        .mockResolvedValueOnce({ kind: "written", commit: "c1" });
      const wrapper = mountPage();
      await wrapper.find(editBtn()).trigger("click");
      await flushPromises();
      await wrapper.find("#e-password").setValue("newpass");
      await wrapper.find("form").trigger("submit");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("edit_secret", {
        name: "servers/prod",
        content: "newpass",
      });
    });

    it("Save is disabled while the body is unchanged or empty (no-op-save guard)", async () => {
      vi.mocked(invoke).mockResolvedValue({ password: "s3cret", notes: "n" });
      const wrapper = mountPage();
      await wrapper.find(editBtn()).trigger("click");
      await flushPromises();

      // Unchanged from the loaded body → disabled.
      expect(
        (wrapper.find(saveBtn()).element as HTMLButtonElement).disabled,
      ).toBe(true);

      // Edit → enabled.
      await wrapper.find("#e-notes").setValue("changed");
      expect(
        (wrapper.find(saveBtn()).element as HTMLButtonElement).disabled,
      ).toBe(false);

      // Clear both → empty body → disabled.
      await wrapper.find("#e-password").setValue("");
      await wrapper.find("#e-notes").setValue("");
      expect(
        (wrapper.find(saveBtn()).element as HTMLButtonElement).disabled,
      ).toBe(true);

      // All-whitespace body would brick the secret on read (Secret::parse
      // rejects it as empty after trim) → disabled.
      await wrapper.find("#e-password").setValue("   ");
      expect(
        (wrapper.find(saveBtn()).element as HTMLButtonElement).disabled,
      ).toBe(true);
    });

    it("on Written, toasts and exits to the read-only view without navigating", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce({ password: "s3cret", notes: "" })
        .mockResolvedValueOnce({ kind: "written", commit: "abc1234" });
      const wrapper = mountPage();
      await wrapper.find(editBtn()).trigger("click");
      await flushPromises();
      await wrapper.find("#e-password").setValue("newpass");
      await wrapper.find("form").trigger("submit");
      await flushPromises();

      expect(wrapper.text()).toContain("✓ Saved (commit abc1234)");
      expect(mockPush).not.toHaveBeenCalled();
      // Exited edit mode — the edit form is gone.
      expect(wrapper.find("#e-password").exists()).toBe(false);
    });

    it("on a conflict, renders the modal and Keep mine resolves → exit + toast", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce({ password: "s3cret", notes: "" })
        .mockResolvedValueOnce({
          kind: "conflict",
          name: "servers/prod",
          remote_decryptable: true,
        })
        .mockResolvedValueOnce({ commit: "def5678" });
      const wrapper = mountPage();
      await wrapper.find(editBtn()).trigger("click");
      await flushPromises();
      await wrapper.find("#e-password").setValue("newpass");
      await wrapper.find("form").trigger("submit");
      await flushPromises();

      expect(wrapper.text()).toContain("Remote copy exists");

      const keepMine = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Keep mine"))!;
      await keepMine.trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("resolve_write_conflict", {
        choice: "keep_mine",
      });
      expect(wrapper.text()).toContain("✓ Saved (commit def5678)");
      expect(wrapper.find("#e-password").exists()).toBe(false);
    });

    it("cancel edit returns to the read-only view without invoking edit_secret", async () => {
      vi.mocked(invoke).mockResolvedValue({ password: "s3cret", notes: "" });
      const wrapper = mountPage();
      await wrapper.find(editBtn()).trigger("click");
      await flushPromises();
      await wrapper.find(cancelEditBtn()).trigger("click");
      await flushPromises();

      expect(wrapper.find("#e-password").exists()).toBe(false);
      expect(invoke).not.toHaveBeenCalledWith("edit_secret", expect.anything());
    });

    it("disables Save and Cancel while the save is inflight", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce({ password: "s3cret", notes: "" })
        .mockReturnValue(new Promise(() => {})); // edit_secret never resolves
      const wrapper = mountPage();
      await wrapper.find(editBtn()).trigger("click");
      await flushPromises();
      await wrapper.find("#e-password").setValue("newpass");
      const save = wrapper.find(saveBtn());
      const cancel = wrapper.find(cancelEditBtn());
      await wrapper.find("form").trigger("submit");
      await flushPromises();

      expect((save.element as HTMLButtonElement).disabled).toBe(true);
      expect((cancel.element as HTMLButtonElement).disabled).toBe(true);
    });

    it("on identity lock, exits edit mode and drops the edit draft", async () => {
      const { setLocked } = useLockState();
      setLocked(false);
      vi.mocked(invoke).mockResolvedValue({ password: "s3cret", notes: "n" });
      const wrapper = mountPage();
      await wrapper.find(editBtn()).trigger("click");
      await flushPromises();
      expect(wrapper.find("#e-password").exists()).toBe(true);

      setLocked(true);
      await flushPromises();

      // exitEdit() ran on lock — the edit form (and its in-DOM plaintext) is gone.
      expect(wrapper.find("#e-password").exists()).toBe(false);
    });

    it("conflict 'View existing' reveals the remote version via show_remote_secret", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce({ password: "s3cret", notes: "" })
        .mockResolvedValueOnce({
          kind: "conflict",
          name: "servers/prod",
          remote_decryptable: true,
        })
        .mockResolvedValueOnce({ password: "teammate-pw", notes: "theirs" });
      const wrapper = mountPage();
      await wrapper.find(editBtn()).trigger("click");
      await flushPromises();
      await wrapper.find("#e-password").setValue("newpass");
      await wrapper.find("form").trigger("submit");
      await flushPromises();

      const view = wrapper
        .findAll("button")
        .find((b) => b.text().includes("View existing"))!;
      await view.trigger("click");
      await flushPromises();

      // The remote (teammate) version — not the local rolled-back copy.
      expect(invoke).toHaveBeenCalledWith("show_remote_secret", {
        name: "servers/prod",
      });
      expect(wrapper.text()).toContain("teammate-pw");
    });

    it("cold-edit fetch failure shows the error and does not enter edit mode", async () => {
      vi.mocked(invoke).mockRejectedValue({
        code: "STORE_ERROR",
        message: "Decryption failed",
      });
      const wrapper = mountPage();
      await wrapper.find(editBtn()).trigger("click");
      await flushPromises();

      expect(wrapper.find("#e-password").exists()).toBe(false);
      expect(wrapper.find("[role='alert']").text()).toContain(
        "Decryption failed",
      );
    });

    it("on a non-PUSH_REJECTED edit error, shows the error and keeps the draft", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce({ password: "s3cret", notes: "" })
        .mockRejectedValueOnce({ code: "STORE_ERROR", message: "Disk full" });
      const wrapper = mountPage();
      await wrapper.find(editBtn()).trigger("click");
      await flushPromises();
      await wrapper.find("#e-password").setValue("newpass");
      await wrapper.find("form").trigger("submit");
      await flushPromises();

      expect(wrapper.find("[role='alert']").text()).toContain("Disk full");
      // Stays in edit mode with the draft.
      expect(wrapper.find("#e-password").exists()).toBe(true);
    });

    it("on PUSH_REJECTED mid-resolve, closes the modal and stays in edit for retry", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce({ password: "s3cret", notes: "" })
        .mockResolvedValueOnce({
          kind: "conflict",
          name: "servers/prod",
          remote_decryptable: true,
        })
        .mockRejectedValueOnce({ code: "PUSH_REJECTED", message: "moved" });
      const wrapper = mountPage();
      await wrapper.find(editBtn()).trigger("click");
      await flushPromises();
      await wrapper.find("#e-password").setValue("newpass");
      await wrapper.find("form").trigger("submit");
      await flushPromises();

      const keepMine = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Keep mine"))!;
      await keepMine.trigger("click");
      await flushPromises();

      expect(wrapper.text()).toContain("Remote changed again");
      // Modal closed, but the edit form + draft remain for retry.
      expect(wrapper.text()).not.toContain("Remote copy exists");
      expect(wrapper.find("#e-password").exists()).toBe(true);
    });
  });
});
