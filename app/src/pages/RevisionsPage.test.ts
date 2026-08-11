// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import type { CommitSigInfo, RevisionView } from "@/api";
import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises, type DOMWrapper } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import RevisionsPage from "./RevisionsPage.vue";

vi.mock("@tauri-apps/api/core");
vi.mock("vue-router", () => ({
  createRouter: vi.fn(),
  createWebHashHistory: vi.fn(),
  useRouter: () => ({ push: vi.fn(), replace: vi.fn(), back: vi.fn() }),
  useRoute: () => ({
    params: { pathMatch: "servers/prod.age" },
    query: {},
    name: "revisions",
    path: "/revisions/servers/prod.age",
    fullPath: "/revisions/servers/prod.age",
  }),
}));

const commit = (over: Partial<CommitSigInfo> = {}): CommitSigInfo => ({
  hash: "abc123def4567890",
  short_hash: "abc123d",
  author: "Alice <alice@example.com>",
  date: "2026-07-01T12:00:00Z",
  subject: "Save secret",
  status: { kind: "unsigned" },
  ignored: false,
  ...over,
});

/** Build a `list_revisions` page envelope (snake_case fields, with base_oid). */
const page = (
  commits: CommitSigInfo[],
  opts: { hasMore?: boolean; baseOid?: string } = {},
) => ({
  commits,
  has_more: opts.hasMore ?? false,
  base_oid: opts.baseOid ?? "deadbeef",
});

describe("RevisionsPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "list_revisions") return Promise.resolve(page([]));
      return Promise.resolve(undefined);
    });
  });

  const findLoadMore = (w: { find: (s: string) => DOMWrapper<Element> }) =>
    w.find('button[aria-label="Load more revisions"]');

  it("requests the revisions of the route's entry path", async () => {
    const wrapper = mountWithApp(RevisionsPage).wrapper;
    await flushPromises();
    // The page names which secret this is the history of (path minus .age).
    expect(wrapper.text()).toContain("servers/prod");
    expect(invoke).toHaveBeenCalledWith("list_revisions", {
      repoId: "test-repo",
      entryPath: "servers/prod.age",
      offset: 0,
      limit: 50,
      baseOid: null,
    });
  });

  it("badges the newest (HEAD) row as current", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) =>
      cmd === "list_revisions"
        ? Promise.resolve(
            page([
              commit({ subject: "newest" }),
              commit({ hash: "h2", subject: "older" }),
            ]),
          )
        : Promise.resolve(undefined),
    );
    const wrapper = mountWithApp(RevisionsPage).wrapper;
    await flushPromises();

    expect(wrapper.text()).toContain("current");
    // Only the HEAD row carries the badge — one occurrence.
    expect(wrapper.text().match(/current/g)).toHaveLength(1);
  });

  it("clicking a row opens the revision detail sheet", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) =>
      cmd === "list_revisions"
        ? Promise.resolve(page([commit()]))
        : Promise.resolve(undefined),
    );
    const wrapper = mountWithApp(RevisionsPage).wrapper;
    await flushPromises();

    await wrapper.find('[role="button"]').trigger("click");
    await flushPromises();

    expect(wrapper.find('[aria-label="Revision detail"]').exists()).toBe(true);
    expect(wrapper.text()).toContain("abc123d");
  });

  describe("pagination", () => {
    it("passes the captured base_oid back on load-more", async () => {
      const page0 = page(
        Array.from({ length: 50 }, (_, i) => commit({ hash: `h${i}` })),
        { hasMore: true, baseOid: "cafebabe" },
      );
      let call = 0;
      vi.mocked(invoke).mockImplementation((cmd: string) => {
        if (cmd === "list_revisions") {
          const result = call === 0 ? page0 : page([commit({ hash: "h50" })]);
          call += 1;
          return Promise.resolve(result);
        }
        return Promise.resolve(undefined);
      });

      const wrapper = mountWithApp(RevisionsPage).wrapper;
      await flushPromises();

      await findLoadMore(wrapper).trigger("click");
      await flushPromises();

      // The load-more call must anchor to page 0's base_oid.
      expect(invoke).toHaveBeenCalledWith("list_revisions", {
        repoId: "test-repo",
        entryPath: "servers/prod.age",
        offset: 50,
        limit: 50,
        baseOid: "cafebabe",
      });
    });

    it("hides load-more when the first page is exhaustive", async () => {
      vi.mocked(invoke).mockImplementation((cmd: string) =>
        cmd === "list_revisions"
          ? Promise.resolve(page([commit()], { hasMore: false }))
          : Promise.resolve(undefined),
      );
      const wrapper = mountWithApp(RevisionsPage).wrapper;
      await flushPromises();
      expect(findLoadMore(wrapper).exists()).toBe(false);
    });
  });

  describe("view states", () => {
    const findButton = (
      w: { findAll: (s: string) => DOMWrapper<Element>[] },
      text: string,
    ) => w.findAll("button").find((b) => b.text().includes(text));

    // Load one revision, open its detail sheet, and click "Show this version"
    // with `view` as the show_revision result.
    async function openAndShow(view: RevisionView) {
      vi.mocked(invoke).mockImplementation((cmd: string) => {
        if (cmd === "list_revisions") return Promise.resolve(page([commit()]));
        if (cmd === "show_revision") return Promise.resolve(view);
        return Promise.resolve(undefined);
      });
      const wrapper = mountWithApp(RevisionsPage).wrapper;
      await flushPromises(); // list loads
      await wrapper.find('[role="button"]').trigger("click"); // open the row
      await flushPromises();
      await findButton(wrapper, "Show this version")!.trigger("click");
      await flushPromises();
      return wrapper;
    }

    it("reveals a decrypted revision under the past-version banner", async () => {
      const wrapper = await openAndShow({
        kind: "decrypted",
        password: "old-pw",
        notes: "old-notes",
        attributes: [],
        has_totp: false,
        attachment: null,
      });
      // the banner marks the revealed value as NOT the current one.
      expect(wrapper.text()).toContain("not your current value");
      // The revealed past value is on screen.
      expect(wrapper.text()).toContain("old-pw");
      expect(wrapper.text()).toContain("old-notes");
    });

    it("shows the undecryptable alert and hides Copy when the revision can't decrypt", async () => {
      const wrapper = await openAndShow({ kind: "undecryptable" });
      expect(wrapper.text()).toContain("Can't decrypt this version");
      // Copy is offered only for idle/revealed — never for undecryptable.
      expect(findButton(wrapper, "Copy this version")).toBeUndefined();
    });

    it("shows the deleted alert and hides Copy when the revision deleted the entry", async () => {
      const wrapper = await openAndShow({ kind: "deleted" });
      expect(wrapper.text()).toContain("This change deleted the entry");
      expect(findButton(wrapper, "Copy this version")).toBeUndefined();
    });

    it("shows the attachment notice (not a blank reveal) for a past attachment revision", async () => {
      const wrapper = await openAndShow({
        kind: "decrypted",
        password: "",
        notes: "",
        attributes: [],
        has_totp: false,
        attachment: { filename: "photo.png", size: 1234 },
      });
      // The honest notice renders (not the old blank reveal box).
      expect(wrapper.text()).toContain("Binary attachment");
      expect(wrapper.text()).toContain("photo.png");
      // No Copy — a past attachment has no copyable password.
      expect(findButton(wrapper, "Copy this version")).toBeUndefined();
    });
  });
});
