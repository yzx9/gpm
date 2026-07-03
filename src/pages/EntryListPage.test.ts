// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import type { Entry, EntryPage } from "@/api";
import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import EntryListPage from "./EntryListPage.vue";

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

/** Wrap entries as a paginated EntryPage response. */
function page(
  entries: Entry[],
  opts: { hasMore?: boolean; total?: number } = {},
): EntryPage {
  return {
    entries,
    total: opts.total ?? entries.length,
    has_more: opts.hasMore ?? false,
  };
}

/** Default successful return values per command (order-independent). */
const defaults: Record<string, unknown> = {
  list_entries: page(sampleEntries),
  search_entries: page(sampleEntries),
  get_authenticity_state: { mode: "off", head_status: { kind: "unsigned" } },
  sync_repo: {
    kind: "fast_forwarded",
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
    return mountWithApp(EntryListPage).wrapper;
  }

  /** Find the "Load more" button by its stable aria-label, if present. */
  function findLoadMore(wrapper: ReturnType<typeof mountPage>) {
    return wrapper.find('button[aria-label="Load more entries"]');
  }

  describe("entry loading", () => {
    it("calls list_entries on mount", async () => {
      mountPage();
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("list_entries", {
        offset: 0,
        limit: 50,
      });
    });

    it("displays entries after loading", async () => {
      when("list_entries", page(sampleEntries));
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
            : Promise.resolve(page(sampleEntries));
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
      when("list_entries", page([]));
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

  describe("copyPassword", () => {
    it("swallows AUTH_CANCELLED silently when the auth overlay is dismissed during copy", async () => {
      when("list_entries", page(sampleEntries));
      // unlocked:false → identity NOT cached → copy's runWithAuth parks on the
      // auth overlay (no singleton to wipe mid-test).
      const { wrapper, lock } = mountWithApp(EntryListPage, {
        unlocked: false,
      });
      await flushPromises();

      await wrapper.find('button[aria-label="Copy password"]').trigger("click");
      await flushPromises(); // parked awaiting auth

      lock.cancelAuth(); // user dismissed the overlay (back)
      await flushPromises();

      // No failure toast — the catch swallowed AUTH_CANCELLED; copy never ran.
      expect(wrapper.text()).not.toContain("Failed:");
    });
  });

  describe("search", () => {
    it("debounces and renders backend search results", async () => {
      when("list_entries", page(sampleEntries));
      when(
        "search_entries",
        page([{ path: "github.com/token.age", name: "github-token" }]),
      );
      const wrapper = mountPage();
      await flushPromises();

      await wrapper.find('input[type="search"]').setValue("git");
      await flushPromises(); // watch schedules the debounce timer
      expect(invoke).not.toHaveBeenCalledWith("search_entries", {
        query: "git",
        offset: 0,
        limit: 50,
      });

      vi.advanceTimersByTime(150);
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("search_entries", {
        query: "git",
        offset: 0,
        limit: 50,
      });
      expect(wrapper.text()).toContain("github-token");
      expect(wrapper.text()).not.toContain("work-email");
    });

    it("shows no matches when the backend returns empty", async () => {
      when("list_entries", page(sampleEntries));
      when("search_entries", page([]));
      const wrapper = mountPage();
      await flushPromises();

      await wrapper.find('input[type="search"]').setValue("zzz");
      await flushPromises();
      vi.advanceTimersByTime(150);
      await flushPromises();

      expect(wrapper.text()).toContain("No matches");
    });

    it("clearing the search refetches browse page 0", async () => {
      // With pagination the WebView no longer holds the full list, so clearing
      // the query issues a fresh browse page-0 fetch (it does not reuse a seed).
      when("list_entries", page(sampleEntries));
      when("search_entries", page([]));
      const wrapper = mountPage();
      await flushPromises();

      await wrapper.find('input[type="search"]').setValue("zzz");
      await flushPromises();
      vi.advanceTimersByTime(150);
      await flushPromises();
      expect(wrapper.text()).toContain("No matches");

      const listBefore = vi
        .mocked(invoke)
        .mock.calls.filter((c) => c[0] === "list_entries").length;
      await wrapper.find('input[type="search"]').setValue("");
      await flushPromises();
      const listAfter = vi
        .mocked(invoke)
        .mock.calls.filter((c) => c[0] === "list_entries").length;

      expect(listAfter).toBeGreaterThan(listBefore); // a fresh browse fetch fired
      expect(wrapper.text()).toContain("github-token");
    });

    it("falls back to browse page 0 + toast on search failure (not 'No matches')", async () => {
      when("list_entries", page(sampleEntries));
      reject("search_entries", { code: "StoreError", message: "boom" });
      const wrapper = mountPage();
      await flushPromises();

      await wrapper.find('input[type="search"]').setValue("git");
      await flushPromises();
      vi.advanceTimersByTime(150);
      await flushPromises();

      expect(wrapper.text()).toContain("github-token"); // browse fallback, not blanked
      expect(wrapper.text()).toContain("boom"); // error toast surfaced
      expect(wrapper.text()).not.toContain("No matches"); // not a misleading empty
    });

    it("only the latest query is searched when typing fast (debounce coalescing)", async () => {
      when("list_entries", page(sampleEntries));
      when(
        "search_entries",
        page([{ path: "github.com/token.age", name: "github-token" }]),
      );
      const wrapper = mountPage();
      await flushPromises();

      // Type "g", then "gi" before the 150 ms debounce fires.
      await wrapper.find('input[type="search"]').setValue("g");
      await flushPromises();
      vi.advanceTimersByTime(149); // "g" debounce not yet fired
      await wrapper.find('input[type="search"]').setValue("gi");
      await flushPromises();
      vi.advanceTimersByTime(150); // only the "gi" debounce fires
      await flushPromises();

      expect(invoke).not.toHaveBeenCalledWith("search_entries", {
        query: "g",
        offset: 0,
        limit: 50,
      });
      expect(invoke).toHaveBeenCalledWith("search_entries", {
        query: "gi",
        offset: 0,
        limit: 50,
      });
    });

    it("refreshes search results after a pull changes the store", async () => {
      when("list_entries", page(sampleEntries));
      when(
        "search_entries",
        page([{ path: "github.com/token.age", name: "github-token" }]),
      );
      const wrapper = mountPage();
      await flushPromises();

      await wrapper.find('input[type="search"]').setValue("git");
      await flushPromises();
      vi.advanceTimersByTime(150);
      await flushPromises();
      expect(wrapper.text()).toContain("github-token");

      // Pull adds an entry; with an active search, results re-run against the store.
      when(
        "list_entries",
        page([...sampleEntries, { path: "new.age", name: "new" }]),
      );
      when("search_entries", page([{ path: "new.age", name: "new-git" }]));
      when("sync_repo", {
        kind: "fast_forwarded",
        changed: true,
        head: "def456",
        authenticity: {
          mode: "off",
          new_commits: [],
          open_issues: [],
          blocked: false,
        },
      });
      await wrapper
        .find('button[aria-label="Sync with remote"]')
        .trigger("click");
      await flushPromises();
      await flushPromises();

      expect(wrapper.text()).toContain("new-git");
    });
  });

  describe("pagination", () => {
    it("appends the next page on load-more", async () => {
      const page0: Entry[] = Array.from({ length: 50 }, (_, i) => ({
        path: `e${i}.age`,
        name: `e${i}`,
      }));
      const page1: Entry[] = [{ path: "e50.age", name: "e50" }];
      vi.mocked(invoke).mockImplementation((cmd: string, args?: unknown) => {
        if (cmd === "list_entries") {
          const offset =
            ((args as Record<string, unknown> | undefined)?.offset as number) ??
            0;
          return Promise.resolve(
            offset === 0
              ? page(page0, { hasMore: true, total: 51 })
              : page(page1, { total: 51 }),
          );
        }
        return Promise.resolve(defaults[cmd]);
      });
      const wrapper = mountPage();
      await flushPromises();

      expect(wrapper.text()).toContain("e0");
      const more = findLoadMore(wrapper);
      expect(more.exists()).toBe(true);
      expect(more.text()).toContain("(1 more)"); // 51 total − 50 loaded

      await more.trigger("click");
      await flushPromises();

      expect(wrapper.text()).toContain("e0"); // page 0 retained (appended, not replaced)
      expect(wrapper.text()).toContain("e50"); // page 1 appended
      expect(invoke).toHaveBeenCalledWith("list_entries", {
        offset: 50,
        limit: 50,
      });
      expect(findLoadMore(wrapper).exists()).toBe(false); // exhausted → button gone
    });

    it("resets to page 0 when the query changes (replaces, does not append)", async () => {
      vi.mocked(invoke).mockImplementation((cmd: string, args?: unknown) => {
        if (cmd === "list_entries") return Promise.resolve(page(sampleEntries));
        if (cmd === "search_entries") {
          const q =
            ((args as Record<string, unknown> | undefined)?.query as string) ??
            "";
          if (q === "foo")
            return Promise.resolve(page([{ path: "foo.age", name: "foo-x" }]));
          if (q === "foobar")
            return Promise.resolve(
              page([{ path: "foobar.age", name: "foobar-y" }]),
            );
        }
        return Promise.resolve(defaults[cmd]);
      });
      const wrapper = mountPage();
      await flushPromises();

      await wrapper.find('input[type="search"]').setValue("foo");
      await flushPromises();
      vi.advanceTimersByTime(150);
      await flushPromises();
      expect(wrapper.text()).toContain("foo-x");

      await wrapper.find('input[type="search"]').setValue("foobar");
      await flushPromises();
      vi.advanceTimersByTime(150);
      await flushPromises();

      expect(wrapper.text()).toContain("foobar-y");
      expect(wrapper.text()).not.toContain("foo-x"); // replaced, not appended
    });

    it("renders no load-more button when the first page is exhaustive", async () => {
      when("list_entries", page(sampleEntries)); // has_more false
      const wrapper = mountPage();
      await flushPromises();

      expect(findLoadMore(wrapper).exists()).toBe(false);
    });

    it("disables the load-more button while a page is loading", async () => {
      const page0: Entry[] = Array.from({ length: 50 }, (_, i) => ({
        path: `e${i}.age`,
        name: `e${i}`,
      }));
      vi.mocked(invoke).mockImplementation((cmd: string, args?: unknown) => {
        if (cmd === "list_entries") {
          const offset =
            ((args as Record<string, unknown> | undefined)?.offset as number) ??
            0;
          if (offset === 0)
            return Promise.resolve(page(page0, { hasMore: true, total: 100 }));
          return new Promise(() => {}); // page 1 never resolves → stays loading
        }
        return Promise.resolve(defaults[cmd]);
      });
      const wrapper = mountPage();
      await flushPromises();

      await findLoadMore(wrapper).trigger("click");
      await flushPromises();

      expect(findLoadMore(wrapper).attributes("disabled")).toBeDefined();
    });
  });

  describe("copyPassword", () => {
    it("calls copy_password and shows success toast", async () => {
      when("list_entries", page(sampleEntries));
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
      when("list_entries", page(sampleEntries));
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
      when("list_entries", page(sampleEntries));
      reject("copy_password", { code: "Err", message: "Copy failed" });
      const wrapper = mountPage();
      await flushPromises();

      await wrapper.find('button[aria-label="Copy password"]').trigger("click");
      await flushPromises();

      expect(wrapper.text()).toContain("Failed: Copy failed");
    });
  });

  describe("syncRepo", () => {
    it("shows 'Already up to date' when no changes", async () => {
      when("sync_repo", {
        kind: "fast_forwarded",
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

      await wrapper
        .find('button[aria-label="Sync with remote"]')
        .trigger("click");
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
            listCall === 1 ? page(sampleEntries) : page(updatedEntries),
          );
        }
        if (cmd === "sync_repo") {
          return Promise.resolve({
            kind: "fast_forwarded",
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

      await wrapper
        .find('button[aria-label="Sync with remote"]')
        .trigger("click");
      await flushPromises();

      expect(wrapper.text()).toContain("Updated to def456");
      expect(wrapper.text()).toContain("new-entry");
    });

    it("shows the divergence modal when diverged (two-step, no checkbox)", async () => {
      when("sync_repo", {
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

      await wrapper
        .find('button[aria-label="Sync with remote"]')
        .trigger("click");
      await flushPromises();

      // Modal surfaces, listing every local-side change category.
      expect(wrapper.text()).toContain("Local and remote have diverged");
      expect(wrapper.text()).toContain("local-only");
      expect(wrapper.text()).toContain("shared");
      expect(wrapper.text()).toContain("notes.txt");

      // No confirm-checkbox anymore — Adopt is immediately enabled.
      expect(wrapper.find('input[type="checkbox"]').exists()).toBe(false);
      const adopt = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Adopt remote"))!;
      expect((adopt.element as HTMLButtonElement).disabled).toBe(false);

      // Tapping it opens the centered confirm (the second step).
      await adopt.trigger("click");
      await flushPromises();
      expect(
        wrapper
          .findAll("button")
          .some((b) => b.text().includes("Discard my commit")),
      ).toBe(true);
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
