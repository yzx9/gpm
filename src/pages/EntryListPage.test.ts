// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { mount } from "@vue/test-utils";
import { flushPromises } from "@vue/test-utils";
import { invoke } from "@tauri-apps/api/core";
import EntryListPage from "./EntryListPage.vue";
import type { Entry } from "../types";

const { mockPush } = vi.hoisted(() => ({
  mockPush: vi.fn(),
}));

vi.mock("@tauri-apps/api/core");
vi.mock("vue-router", () => ({
  createRouter: vi.fn(),
  createWebHashHistory: vi.fn(),
  useRouter: () => ({
    push: mockPush,
    replace: vi.fn(),
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

const sampleEntries: Entry[] = [
  { path: "github.com/token.age", name: "github-token" },
  { path: "email/work.age", name: "work-email" },
  { path: "servers/prod.age", name: "prod-server" },
];

describe("EntryListPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  function mountPage() {
    return mount(EntryListPage);
  }

  describe("entry loading", () => {
    it("calls list_entries on mount", async () => {
      vi.mocked(invoke).mockResolvedValue(sampleEntries);
      mountPage();
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("list_entries");
    });

    it("displays entries after loading", async () => {
      vi.mocked(invoke).mockResolvedValue(sampleEntries);
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).toContain("github-token");
      expect(wrapper.text()).toContain("work-email");
      expect(wrapper.text()).toContain("prod-server");
    });

    it("shows error when loading fails", async () => {
      vi.mocked(invoke).mockRejectedValue({
        code: "StoreError",
        message: "Store not found",
      });
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.find("[role='alert']").text()).toContain(
        "Store not found",
      );
    });

    it("shows retry button on error", async () => {
      vi.mocked(invoke)
        .mockRejectedValueOnce({ code: "Err", message: "fail" })
        .mockResolvedValueOnce(sampleEntries);
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.find(".btn-retry").exists()).toBe(true);
      await wrapper.find(".btn-retry").trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledTimes(2);
      expect(wrapper.text()).toContain("github-token");
    });

    it("shows empty state when no entries", async () => {
      vi.mocked(invoke).mockResolvedValue([]);
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).toContain("No passwords yet");
    });

    it("shows loading spinner while loading", async () => {
      // Return a promise that never resolves to keep loading state
      vi.mocked(invoke).mockReturnValue(new Promise(() => {}));
      const wrapper = mountPage();
      // Flush Vue's reactive render so loading=true appears in DOM
      await flushPromises();

      expect(wrapper.text()).toContain("Loading entries...");
    });
  });

  describe("search/filter", () => {
    it("filters entries by search input", async () => {
      vi.mocked(invoke).mockResolvedValue(sampleEntries);
      const wrapper = mountPage();
      await flushPromises();

      await wrapper.find('input[type="search"]').setValue("git");
      await flushPromises();

      // Only github-token should be visible
      expect(wrapper.text()).toContain("github-token");
      expect(wrapper.text()).not.toContain("work-email");
    });

    it("shows no matches message when search has no results", async () => {
      vi.mocked(invoke).mockResolvedValue(sampleEntries);
      const wrapper = mountPage();
      await flushPromises();

      await wrapper.find('input[type="search"]').setValue("nonexistent");
      await flushPromises();

      expect(wrapper.text()).toContain("No matches");
    });
  });

  describe("copyPassword", () => {
    it("calls copy_password and shows success toast", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(sampleEntries) // list_entries
        .mockResolvedValueOnce({
          entry_name: "github-token",
          cleared_after_secs: 45,
        });
      const wrapper = mountPage();
      await flushPromises();

      await wrapper.find('button[aria-label="Copy password"]').trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("copy_password", {
        entryPath: "github.com/token.age",
      });
      expect(wrapper.text()).toContain("✓ Copied github-token");
    });

    it("auto-clears toast after 3 seconds", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(sampleEntries)
        .mockResolvedValueOnce({
          entry_name: "github-token",
          cleared_after_secs: 45,
        });
      const wrapper = mountPage();
      await flushPromises();

      await wrapper.find('button[aria-label="Copy password"]').trigger("click");
      await flushPromises();

      expect(wrapper.text()).toContain("✓ Copied");

      vi.advanceTimersByTime(3000);
      await flushPromises();

      expect(wrapper.text()).not.toContain("✓ Copied");
    });

    it("shows error toast on copy failure", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(sampleEntries)
        .mockRejectedValueOnce({ code: "Err", message: "Copy failed" });
      const wrapper = mountPage();
      await flushPromises();

      await wrapper.find('button[aria-label="Copy password"]').trigger("click");
      await flushPromises();

      expect(wrapper.text()).toContain("Failed: Copy failed");
    });
  });

  describe("pullRepo", () => {
    it("shows 'Already up to date' when no changes", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(sampleEntries) // list_entries on mount
        .mockResolvedValueOnce({ changed: false, head: "abc" }); // pull_repo
      const wrapper = mountPage();
      await flushPromises();

      await wrapper.find('button[aria-label="Pull updates"]').trigger("click");
      await flushPromises();

      expect(wrapper.text()).toContain("Already up to date");
    });

    it("reloads entries and shows update message when changed", async () => {
      const updatedEntries: Entry[] = [
        ...sampleEntries,
        { path: "new.age", name: "new-entry" },
      ];
      vi.mocked(invoke)
        .mockResolvedValueOnce(sampleEntries) // initial load
        .mockResolvedValueOnce({ changed: true, head: "def456" }) // pull_repo
        .mockResolvedValueOnce(updatedEntries); // reload after pull
      const wrapper = mountPage();
      await flushPromises();

      await wrapper.find('button[aria-label="Pull updates"]').trigger("click");
      await flushPromises();

      expect(wrapper.text()).toContain("Updated to def456");
      expect(wrapper.text()).toContain("new-entry");
    });
  });

  describe("resetConfig", () => {
    it("calls reset_config and navigates when confirmed", async () => {
      vi.mocked(invoke)
        .mockResolvedValueOnce(sampleEntries) // list_entries
        .mockResolvedValueOnce(undefined); // reset_config
      vi.mocked(globalThis.confirm).mockReturnValue(true);
      const wrapper = mountPage();
      await flushPromises();

      await wrapper
        .find('button[aria-label="Reset configuration"]')
        .trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("reset_config");
      expect(mockPush).toHaveBeenCalledWith({ name: "setup" });
    });

    it("does nothing when user cancels confirm dialog", async () => {
      vi.mocked(invoke).mockResolvedValue(sampleEntries);
      vi.mocked(globalThis.confirm).mockReturnValue(false);
      const wrapper = mountPage();
      await flushPromises();

      const invokeCount = (invoke as ReturnType<typeof vi.fn>).mock.calls
        .length;
      await wrapper
        .find('button[aria-label="Reset configuration"]')
        .trigger("click");
      await flushPromises();

      // No new invoke calls beyond the initial list_entries
      expect((invoke as ReturnType<typeof vi.fn>).mock.calls.length).toBe(
        invokeCount,
      );
    });
  });
});
