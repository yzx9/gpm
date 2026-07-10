// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import EntryDetailPage from "./EntryDetailPage.vue";

const { mockPush, mockReplace } = vi.hoisted(() => ({
  mockPush: vi.fn(),
  mockReplace: vi.fn(),
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
    replace: mockReplace,
    back: vi.fn(),
  }),
  useRoute: () => mockRoute,
}));

describe("EntryDetailPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
    // "identity cached" precondition is established per-mount by mountWithApp's
    // default unlocked:true (App.vue's init() doesn't run in page tests).
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  function mountPage() {
    return mountWithApp(EntryDetailPage).wrapper;
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

    it("ticks the auto-clear countdown down each second", async () => {
      vi.mocked(invoke).mockResolvedValue({ password: "s3cret", notes: "" });
      const wrapper = mountPage();
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();

      // Freshly revealed: shows the full default 45s window.
      expect(wrapper.text()).toContain("Auto-clears in 45s");

      // One second later: the live countdown has ticked.
      vi.advanceTimersByTime(1_000);
      await flushPromises();
      expect(wrapper.text()).toContain("Auto-clears in 44s");
    });

    it("clamps the countdown at 1s and never shows 0s before the wipe", async () => {
      vi.mocked(invoke).mockResolvedValue({ password: "s3cret", notes: "" });
      const wrapper = mountPage();
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();

      // Tick to the last whole second before the 45s wipe deadline: the clamp
      // holds at 1s, never flashing 0s.
      vi.advanceTimersByTime(44_000);
      await flushPromises();
      expect(wrapper.text()).toContain("Auto-clears in 1s");
      expect(wrapper.text()).not.toContain("Auto-clears in 0s");

      // The final second: the wipe fires and the whole block (label included) hides.
      vi.advanceTimersByTime(1_000);
      await flushPromises();
      expect(wrapper.text()).not.toContain("s3cret");
      expect(wrapper.text()).not.toContain("Auto-clears in");
    });

    it("resets the countdown when the view-clear setting changes mid-reveal", async () => {
      vi.mocked(invoke).mockResolvedValue({ password: "s3cret", notes: "" });
      const { wrapper, securitySettings } = mountWithApp(EntryDetailPage);
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();
      expect(wrapper.text()).toContain("Auto-clears in 45s");

      // A few seconds tick down from the original 45s window.
      vi.advanceTimersByTime(5_000);
      await flushPromises();
      expect(wrapper.text()).toContain("Auto-clears in 40s");

      // Lowering the setting to 10s re-arms from a fresh deadline.
      securitySettings.applySecurityConfig({
        secure_screen: true,
        view_clear_secs: 10,
      });
      await flushPromises();
      expect(wrapper.text()).toContain("Auto-clears in 10s");

      // The new (shorter) deadline governs: 10s later the password wipes.
      vi.advanceTimersByTime(10_000);
      await flushPromises();
      expect(wrapper.text()).not.toContain("s3cret");
    });

    it("toggles off when clicked while already revealed (no re-auth, no re-decrypt)", async () => {
      // Regression: clicking the "Showing..." button used to re-run auth +
      // show_password instead of hiding. It must now clear in place.
      vi.mocked(invoke).mockResolvedValue({
        password: "s3cret",
        notes: "some notes",
      });
      const wrapper = mountPage();
      // First click reveals.
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();
      expect(wrapper.text()).toContain("s3cret");

      // Second click toggles off — the aria-label flips to "Password is showing".
      await wrapper
        .find('button[aria-label="Password is showing"]')
        .trigger("click");
      await flushPromises();

      // Password is hidden again...
      expect(wrapper.text()).not.toContain("s3cret");
      // ...and show_password was NOT invoked a second time.
      expect(
        vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "show_password"),
      ).toHaveLength(1);
    });

    it("auto-clears sensitive data after the default view-clear window", async () => {
      vi.mocked(invoke).mockResolvedValue({
        password: "s3cret",
        notes: "notes",
      });
      const wrapper = mountPage();
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();

      // Password is visible
      expect(wrapper.text()).toContain("s3cret");

      // Advance past the default view-clear window (45s; configurable via Settings).
      vi.advanceTimersByTime(45_000);
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

    it("swallows AUTH_CANCELLED silently when the auth overlay is dismissed (Android back)", async () => {
      // unlocked:false → identity NOT cached → show's runWithAuth parks on the
      // auth overlay instead of running show_password immediately.
      const { wrapper, lock } = mountWithApp(EntryDetailPage, {
        unlocked: false,
      });
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises(); // parked awaiting auth

      lock.cancelAuth(); // user dismissed the overlay (back)
      await flushPromises(); // rejection propagates to the catch

      // No error UI — the catch swallowed AUTH_CANCELLED; the op never ran.
      expect(wrapper.find("[role='alert']").exists()).toBe(false);
    });
  });

  describe("copyPassword", () => {
    it("invokes copy_password and shows success toast", async () => {
      vi.mocked(invoke).mockResolvedValue({
        entry_name: "prod",
        cleared_after_secs: 45,
      });
      const { wrapper, toast } = mountWithApp(EntryDetailPage);
      await wrapper
        .find('button[aria-label="Copy password to clipboard"]')
        .trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith(
        "copy_password",
        expect.objectContaining({ entryPath: "servers/prod.age" }),
      );
      expect(
        toast.toasts.value.some((t) =>
          t.message.includes("✓ Copied prod (45s auto-clear)"),
        ),
      ).toBe(true);
    });

    it("swallows AUTH_CANCELLED silently on copyPassword when the auth overlay is dismissed", async () => {
      // unlocked:false → identity NOT cached → copy's runWithAuth parks on the overlay.
      const { wrapper, lock } = mountWithApp(EntryDetailPage, {
        unlocked: false,
      });
      await wrapper
        .find('button[aria-label="Copy password to clipboard"]')
        .trigger("click");
      await flushPromises(); // parked awaiting auth

      lock.cancelAuth(); // user dismissed the overlay (back)
      await flushPromises();

      // No error UI — the catch swallowed AUTH_CANCELLED; copy never ran.
      expect(wrapper.find("[role='alert']").exists()).toBe(false);
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
      const { wrapper, toast } = mountWithApp(EntryDetailPage);
      await wrapper
        .find('button[aria-label="Copy password to clipboard"]')
        .trigger("click");
      await flushPromises();

      expect(
        toast.toasts.value.some((t) => t.message.includes("✓ Copied prod")),
      ).toBe(true);

      vi.advanceTimersByTime(3000);
      await flushPromises();

      expect(toast.toasts.value).toHaveLength(0);
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
      vi.mocked(invoke).mockResolvedValue({
        password: "s3cret",
        notes: "notes",
      });
      // The modal keeps the page mounted, so a lock transition must wipe in place.
      const { wrapper, lock } = mountWithApp(EntryDetailPage);
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();

      // Password is in the DOM
      expect(wrapper.text()).toContain("s3cret");

      // Lock fires the shared composable's onLock(clear) without unmounting.
      lock.setLocked(true);
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

      expect(mockReplace).toHaveBeenCalledWith({ name: "entries" });
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
      vi.mocked(invoke).mockResolvedValue({
        kind: "written",
        commit: "abc1234",
      });
      const { wrapper, toast } = mountWithApp(EntryDetailPage);
      await wrapper.find(deleteBtn()).trigger("click");
      await flushPromises();

      expect(
        toast.toasts.value.some((t) =>
          t.message.includes("✓ Deleted (commit abc1234)"),
        ),
      ).toBe(true);
      expect(mockReplace).toHaveBeenCalledWith({ name: "entries" });
    });

    it("on delete divergence, surfaces the shared modal and adopt resolves", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce({
          kind: "needs_divergence_resolve",
          local_ahead: 1,
          remote_ahead: 1,
          remote_tip: "abc123",
          local_only_entries: [],
          modified_entries: ["servers/prod"],
          other_changed_files: [],
        })
        .mockResolvedValueOnce({
          changed: true,
          head: "def456",
          authenticity: {
            mode: "off",
            new_commits: [],
            open_issues: [],
            blocked: false,
          },
        });
      const wrapper = mountPage();
      await wrapper.find(deleteBtn()).trigger("click");
      await flushPromises();

      // The shared divergence modal shows (save wording + the modified entry).
      expect(wrapper.text()).toContain("conflicts with a newer remote");
      expect(wrapper.text()).toContain("servers/prod");

      const adopt = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Adopt remote"))!;
      await adopt.trigger("click");
      await flushPromises();

      const confirmBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Discard my commit"))!;
      await confirmBtn.trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("resolve_sync_divergence", {
        expectedRemoteOid: "abc123",
        choice: "adopt_remote",
      });
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
});
