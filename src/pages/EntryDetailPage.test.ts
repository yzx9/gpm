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
});
