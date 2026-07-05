// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import type { CommitSigInfo } from "@/api";
import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises } from "@vue/test-utils";
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

const commit: CommitSigInfo = {
  hash: "abc123def4567890",
  short_hash: "abc123d",
  author: "Alice <alice@example.com>",
  date: "2026-07-01T12:00:00Z",
  subject: "Initial commit",
  status: { kind: "unsigned" },
  ignored: false,
};

describe("HistoryPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "list_commit_signatures") return Promise.resolve([commit]);
      return Promise.resolve(undefined);
    });
  });

  // Row click target is the <li role="button">; clicking opens the detail
  // modal (sets `selected`). Smoke test that the press-target wiring works.
  it("clicking a commit row opens the detail modal", async () => {
    const wrapper = mountWithApp(HistoryPage).wrapper;
    await flushPromises();

    expect(wrapper.text()).toContain("Initial commit");

    const row = wrapper.find('[role="button"]');
    expect(row.exists()).toBe(true);
    await row.trigger("click");
    await flushPromises();

    const modal = wrapper.find('[aria-label="Commit detail"]');
    expect(modal.exists()).toBe(true);
    expect(wrapper.text()).toContain("abc123d");
  });
});
