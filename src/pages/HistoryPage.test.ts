// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import type { CommitSigInfo } from "@/api";
import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises, type DOMWrapper } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import HistoryPage from "./HistoryPage.vue";

vi.mock("@tauri-apps/api/core");
vi.mock("vue-router", () => ({
  createRouter: vi.fn(),
  createWebHashHistory: vi.fn(),
  useRouter: () => ({ push: vi.fn(), replace: vi.fn(), back: vi.fn() }),
  useRoute: () => ({
    params: {},
    query: {},
    name: "",
    path: "/",
    fullPath: "/",
  }),
}));

const commit = (over: Partial<CommitSigInfo> = {}): CommitSigInfo => ({
  hash: "abc123def4567890",
  short_hash: "abc123d",
  author: "Alice <alice@example.com>",
  date: "2026-07-01T12:00:00Z",
  subject: "Initial commit",
  status: { kind: "unsigned" },
  ignored: false,
  ...over,
});

/** Build a `list_commit_signatures` page envelope. */
const page = (commits: CommitSigInfo[], hasMore = false) => ({
  commits,
  has_more: hasMore,
});

describe("HistoryPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "list_commit_signatures") return Promise.resolve(page([]));
      return Promise.resolve(undefined);
    });
  });

  const findLoadMore = (w: { find: (s: string) => DOMWrapper<Element> }) =>
    w.find('button[aria-label="Load more commits"]');

  it("clicking a commit row opens the detail modal", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) =>
      cmd === "list_commit_signatures"
        ? Promise.resolve(page([commit()]))
        : Promise.resolve(undefined),
    );
    const wrapper = mountWithApp(HistoryPage).wrapper;
    await flushPromises();

    expect(wrapper.text()).toContain("Initial commit");
    const row = wrapper.find('[role="button"]');
    expect(row.exists()).toBe(true);
    await row.trigger("click");
    await flushPromises();

    expect(wrapper.find('[aria-label="Commit detail"]').exists()).toBe(true);
    expect(wrapper.text()).toContain("abc123d");
  });

  describe("pagination", () => {
    it("appends the next page on load-more (offset advances, button hides at end)", async () => {
      const page0 = Array.from({ length: 50 }, (_, i) =>
        commit({
          hash: `h${i}`,
          short_hash: `h${i}`.slice(0, 7),
          subject: `c${i}`,
        }),
      );
      const page1 = [commit({ hash: "h50", subject: "c50" })];
      let call = 0;
      vi.mocked(invoke).mockImplementation((cmd: string) => {
        if (cmd === "list_commit_signatures") {
          const result = call === 0 ? page(page0, true) : page(page1, false);
          call += 1;
          return Promise.resolve(result);
        }
        return Promise.resolve(undefined);
      });

      const wrapper = mountWithApp(HistoryPage).wrapper;
      await flushPromises();

      expect(findLoadMore(wrapper).exists()).toBe(true);
      expect(wrapper.findAll('[role="button"]')).toHaveLength(50);

      await findLoadMore(wrapper).trigger("click");
      await flushPromises();

      // page0 (50) + page1 (1) = 51 rows; second page exhausted the list.
      expect(wrapper.findAll('[role="button"]')).toHaveLength(51);
      expect(findLoadMore(wrapper).exists()).toBe(false);
      expect(invoke).toHaveBeenCalledWith("list_commit_signatures", {
        offset: 50,
        limit: 50,
      });
    });

    it("renders no load-more button when the first page is exhaustive", async () => {
      vi.mocked(invoke).mockImplementation((cmd: string) =>
        cmd === "list_commit_signatures"
          ? Promise.resolve(page([commit()], false))
          : Promise.resolve(undefined),
      );
      const wrapper = mountWithApp(HistoryPage).wrapper;
      await flushPromises();

      expect(findLoadMore(wrapper).exists()).toBe(false);
    });

    it("disables the load-more button while a page is loading", async () => {
      const fullPage = page(
        Array.from({ length: 50 }, (_, i) => commit({ hash: `h${i}` })),
        true,
      );
      let first = true;
      vi.mocked(invoke).mockImplementation((cmd: string) => {
        if (cmd === "list_commit_signatures") {
          if (first) {
            first = false;
            return Promise.resolve(fullPage);
          }
          return new Promise(() => {
            /* load-more hangs → loading stays true */
          });
        }
        return Promise.resolve(undefined);
      });

      const wrapper = mountWithApp(HistoryPage).wrapper;
      await flushPromises();
      const btn = findLoadMore(wrapper);
      expect(btn.exists()).toBe(true);

      await btn.trigger("click");
      await flushPromises();

      expect(findLoadMore(wrapper).attributes("disabled")).toBeDefined();
    });

    it("refreshes the row in place after ignore (no list reset)", async () => {
      const target = commit({
        hash: "ign1",
        short_hash: "ign1abc",
        status: { kind: "untrusted_key", signer_fp: "SHA256:abc" },
      });
      vi.mocked(invoke).mockImplementation((cmd: string) => {
        if (cmd === "list_commit_signatures")
          return Promise.resolve(page([target]));
        if (cmd === "ignore_commit_issue")
          return Promise.resolve({ ...target, ignored: true });
        return Promise.resolve(undefined);
      });

      const wrapper = mountWithApp(HistoryPage).wrapper;
      await flushPromises();

      // Open the detail sheet and click "Ignore this issue".
      await wrapper.find('[role="button"]').trigger("click");
      await flushPromises();
      const ignoreBtn = wrapper
        .findAll("button")
        .find((b) => /ignore/i.test(b.text()));
      expect(ignoreBtn).toBeTruthy();
      // Drop the mount-time list call so we can assert ignore triggers no reset.
      vi.mocked(invoke).mockClear();
      await ignoreBtn!.trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("ignore_commit_issue", {
        commit: "ign1",
      });
      // In-place refresh must NOT reset the list (no new list_commit_signatures).
      const listCalls = vi
        .mocked(invoke)
        .mock.calls.filter((c) => c[0] === "list_commit_signatures");
      expect(listCalls).toHaveLength(0);
      // The row now reflects the ignored badge.
      expect(wrapper.text()).toContain("ignored");
    });

    it("toasts and keeps loaded commits when load-more fails", async () => {
      const page0 = page(
        Array.from({ length: 50 }, (_, i) => commit({ hash: `h${i}` })),
        true,
      );
      let first = true;
      vi.mocked(invoke).mockImplementation((cmd: string) => {
        if (cmd === "list_commit_signatures") {
          if (first) {
            first = false;
            return Promise.resolve(page0);
          }
          return Promise.reject(new Error("boom"));
        }
        return Promise.resolve(undefined);
      });

      const { wrapper, toast } = mountWithApp(HistoryPage);
      await flushPromises();
      const dangerSpy = vi.spyOn(toast.toast, "danger");
      await findLoadMore(wrapper).trigger("click");
      await flushPromises();

      // page0 stays loaded; a danger toast surfaces; load-more is still available.
      expect(wrapper.findAll('[role="button"]')).toHaveLength(50);
      expect(findLoadMore(wrapper).exists()).toBe(true);
      expect(dangerSpy).toHaveBeenCalledWith(expect.any(String));
    });

    it("surfaces an error and clears the list when the first page fails", async () => {
      vi.mocked(invoke).mockImplementation((cmd: string) =>
        cmd === "list_commit_signatures"
          ? Promise.reject(new Error("boom"))
          : Promise.resolve(undefined),
      );
      const wrapper = mountWithApp(HistoryPage).wrapper;
      await flushPromises();

      expect(wrapper.findAll('[role="button"]')).toHaveLength(0);
      expect(wrapper.text()).toContain("boom");
      expect(findLoadMore(wrapper).exists()).toBe(false);
    });
  });
});
