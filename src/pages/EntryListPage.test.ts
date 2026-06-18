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

/** Default successful return values per command (order-independent). */
const defaults: Record<string, unknown> = {
  list_entries: sampleEntries,
  get_authenticity_state: { mode: "off", head_status: { kind: "unsigned" } },
  pull_repo: {
    changed: false,
    head: "abc",
    authenticity: {
      mode: "off",
      new_commits: [],
      open_issues: [],
      blocked: false,
    },
  },
  copy_password: { entry_name: "x", cleared_after_secs: 30 },
};

describe("EntryListPage", () => {
  // Per-command overrides: value to resolve, or `{ reject: payload }` to reject.
  const overrides: Record<string, { value?: unknown; reject?: unknown }> = {};

  function when(cmd: string, value: unknown) {
    overrides[cmd] = { value };
  }
  function reject(cmd: string, payload: unknown) {
    overrides[cmd] = { reject: payload };
  }

  beforeEach(() => {
    vi.clearAllMocks();
    for (const k of Object.keys(overrides)) delete overrides[k];
    vi.useFakeTimers();
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd in overrides) {
        const o = overrides[cmd];
        if (o && o.reject !== undefined) return Promise.reject(o.reject);
        return Promise.resolve(o ? o.value : defaults[cmd]);
      }
      return Promise.resolve(defaults[cmd]);
    });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  function mountPage() {
    return mount(EntryListPage);
  }

  describe("entry loading", () => {
    it("calls list_entries on mount", async () => {
      mountPage();
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("list_entries");
    });

    it("displays entries after loading", async () => {
      when("list_entries", sampleEntries);
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).toContain("github-token");
      expect(wrapper.text()).toContain("work-email");
      expect(wrapper.text()).toContain("prod-server");
    });

    it("shows error when loading fails", async () => {
      reject("list_entries", {
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
      // First list_entries rejects; the retry resolves with entries.
      let listCall = 0;
      vi.mocked(invoke).mockImplementation((cmd: string) => {
        if (cmd === "list_entries") {
          listCall += 1;
          return listCall === 1
            ? Promise.reject({ code: "Err", message: "fail" })
            : Promise.resolve(sampleEntries);
        }
        if (cmd in overrides) {
          const o = overrides[cmd];
          if (o && o.reject !== undefined) return Promise.reject(o.reject);
          return Promise.resolve(o ? o.value : defaults[cmd]);
        }
        return Promise.resolve(defaults[cmd]);
      });
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.find(".btn-retry").exists()).toBe(true);
      await wrapper.find(".btn-retry").trigger("click");
      await flushPromises();

      expect(wrapper.text()).toContain("github-token");
    });

    it("shows empty state when no entries", async () => {
      when("list_entries", []);
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).toContain("No passwords yet");
    });

    it("shows loading spinner while loading", async () => {
      // list_entries never resolves → loading stays true.
      when("list_entries", new Promise(() => {}));
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).toContain("Loading entries...");
    });
  });

  describe("search/filter", () => {
    it("filters entries by search input", async () => {
      when("list_entries", sampleEntries);
      const wrapper = mountPage();
      await flushPromises();

      await wrapper.find('input[type="search"]').setValue("git");
      await flushPromises();

      expect(wrapper.text()).toContain("github-token");
      expect(wrapper.text()).not.toContain("work-email");
    });

    it("shows no matches message when search has no results", async () => {
      when("list_entries", sampleEntries);
      const wrapper = mountPage();
      await flushPromises();

      await wrapper.find('input[type="search"]').setValue("nonexistent");
      await flushPromises();

      expect(wrapper.text()).toContain("No matches");
    });
  });

  describe("copyPassword", () => {
    it("calls copy_password and shows success toast", async () => {
      when("list_entries", sampleEntries);
      when("copy_password", {
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
      when("list_entries", sampleEntries);
      when("copy_password", {
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
      when("list_entries", sampleEntries);
      reject("copy_password", { code: "Err", message: "Copy failed" });
      const wrapper = mountPage();
      await flushPromises();

      await wrapper.find('button[aria-label="Copy password"]').trigger("click");
      await flushPromises();

      expect(wrapper.text()).toContain("Failed: Copy failed");
    });
  });

  describe("pullRepo", () => {
    it("shows 'Already up to date' when no changes", async () => {
      when("pull_repo", {
        changed: false,
        head: "abc",
        authenticity: {
          mode: "off",
          new_commits: [],
          open_issues: [],
          blocked: false,
        },
      });
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
      let listCall = 0;
      vi.mocked(invoke).mockImplementation((cmd: string) => {
        if (cmd === "list_entries") {
          listCall += 1;
          return Promise.resolve(
            listCall === 1 ? sampleEntries : updatedEntries,
          );
        }
        if (cmd === "pull_repo") {
          return Promise.resolve({
            changed: true,
            head: "def456",
            authenticity: {
              mode: "off",
              new_commits: [],
              open_issues: [],
              blocked: false,
            },
          });
        }
        if (cmd in overrides) {
          const o = overrides[cmd];
          if (o && o.reject !== undefined) return Promise.reject(o.reject);
          return Promise.resolve(o ? o.value : defaults[cmd]);
        }
        return Promise.resolve(defaults[cmd]);
      });
      const wrapper = mountPage();
      await flushPromises();

      await wrapper.find('button[aria-label="Pull updates"]').trigger("click");
      await flushPromises();

      expect(wrapper.text()).toContain("Updated to def456");
      expect(wrapper.text()).toContain("new-entry");
    });

    it("shows the divergence modal when diverged", async () => {
      when("pull_repo", {
        kind: "diverged",
        local_ahead: 2,
        remote_ahead: 1,
        remote_tip: "deadbeefdeadbeef",
        local_only_entries: ["local-only"],
        modified_entries: ["shared"],
        other_changed_files: ["notes.txt"],
      });
      const wrapper = mountPage();
      await flushPromises();

      await wrapper.find('button[aria-label="Pull updates"]').trigger("click");
      await flushPromises();

      // Modal surfaces, listing every local-side change category.
      expect(wrapper.text()).toContain("Local and remote have diverged");
      expect(wrapper.text()).toContain("local-only");
      expect(wrapper.text()).toContain("shared");
      expect(wrapper.text()).toContain("notes.txt");

      // Adopt is gated behind the confirmation checkbox.
      const checkbox = wrapper.find('input[type="checkbox"]');
      expect(checkbox.exists()).toBe(true);
      expect(wrapper.find(".btn-danger").attributes("disabled")).toBeDefined();

      await checkbox.setValue(true);
      expect(
        wrapper.find(".btn-danger").attributes("disabled"),
      ).toBeUndefined();
    });
  });

  describe("settings navigation", () => {
    it("navigates to settings page when settings button clicked", async () => {
      const wrapper = mountPage();
      await flushPromises();

      await wrapper.find('button[aria-label="Settings"]').trigger("click");
      await flushPromises();

      expect(mockPush).toHaveBeenCalledWith({ name: "settings" });
    });
  });

  describe("authenticity badge", () => {
    it("opens the history page when the badge is tapped", async () => {
      const wrapper = mountPage();
      await flushPromises();

      await wrapper.find('button[aria-label^="Signature"]').trigger("click");
      await flushPromises();

      expect(mockPush).toHaveBeenCalledWith({ name: "history" });
    });
  });
});
