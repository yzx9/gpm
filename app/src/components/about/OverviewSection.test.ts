// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import OverviewSection from "./OverviewSection.vue";

vi.mock("@tauri-apps/api/core");
vi.mock("@/utils/open-external", () => ({
  openExternal: vi.fn().mockResolvedValue(undefined),
}));

const RELEASES_URL = "https://github.com/yzx9/gpm/releases/latest";
// Resolve after import so the module mock above is in place.
const { openExternal } = await import("@/utils/open-external");

describe("OverviewSection (RFC R090 update check)", () => {
  // Per-command overrides; `get_update_status` is driven per test.
  const overrides: Record<string, unknown> = {};

  function installMock() {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_app_config") return Promise.resolve({});
      if (cmd === "acknowledge_update") return Promise.resolve();
      if (cmd === "set_update_check") return Promise.resolve({});
      if (cmd in overrides) return Promise.resolve(overrides[cmd]);
      return Promise.resolve(undefined);
    });
  }

  beforeEach(() => {
    vi.clearAllMocks();
    for (const k of Object.keys(overrides)) delete overrides[k];
    installMock();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  function mountSection() {
    return mountWithApp(OverviewSection).wrapper;
  }

  it("renders no update dot when up to date", async () => {
    overrides.get_update_status = {
      available: false,
      unacknowledged: false,
      latest_version: null,
    };
    const wrapper = mountSection();
    await flushPromises();

    expect(wrapper.find(".update-dot").exists()).toBe(false);
    // The version entry stays a quiet button even with nothing to show.
    expect(wrapper.find(".version-btn").exists()).toBe(true);
  });

  it("lights the dot and routes the version tap to the release page", async () => {
    overrides.get_update_status = {
      available: true,
      unacknowledged: true,
      latest_version: "v0.19.0",
    };
    const wrapper = mountSection();
    await flushPromises();

    expect(wrapper.find(".update-dot").exists()).toBe(true);
    // No standalone update link on the page anymore — the dialog owns it.
    expect(wrapper.find(".update-link").exists()).toBe(false);

    // Version tap opens the dialog; its primary action opens the release page.
    await wrapper.find(".version-btn").trigger("click");
    await flushPromises();
    const primary = wrapper.findAll(".dialog-actions button")[0]!;
    expect(primary.text()).toBe("Go to download page");
    await primary.trigger("click");
    await flushPromises();

    expect(openExternal).toHaveBeenCalledWith(RELEASES_URL);
  });

  it("keeps the About-page dot lit even after the update is acknowledged", async () => {
    // `unacknowledged: false` but `available: true`: the About-page dot ignores
    // the ack (RFC R090) — only the Settings-entry dot falls quiet.
    overrides.get_update_status = {
      available: true,
      unacknowledged: false,
      latest_version: "v0.19.0",
    };
    const wrapper = mountSection();
    await flushPromises();

    expect(wrapper.find(".update-dot").exists()).toBe(true);
    // Nothing unacknowledged ⇒ no ack call this visit.
    expect(invoke).not.toHaveBeenCalledWith("acknowledge_update");
  });

  it("acknowledges the release on mount when it is unacknowledged", async () => {
    overrides.get_update_status = {
      available: true,
      unacknowledged: true,
      latest_version: "v0.19.0",
    };
    mountSection();
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("acknowledge_update");
  });

  it("toggles the update-check pref from the version dialog's footer link", async () => {
    overrides.get_update_status = {
      available: false,
      unacknowledged: false,
      latest_version: "v0.19.0",
    };
    const wrapper = mountSection();
    await flushPromises();

    await wrapper.find(".version-btn").trigger("click");
    await flushPromises();
    await wrapper.find(".pref-link").trigger("click");
    await flushPromises();

    // get_app_config returns {} ⇒ the pref reads as default-on, so the link
    // offers to disable it.
    expect(invoke).toHaveBeenCalledWith("set_update_check", { enabled: false });
  });

  it("re-reads the cached status when the dialog closes", async () => {
    overrides.get_update_status = {
      available: false,
      unacknowledged: false,
      latest_version: "v0.19.0",
    };
    const wrapper = mountSection();
    await flushPromises();

    await wrapper.find(".version-btn").trigger("click");
    await flushPromises();
    // Cancel closes; the dot source is refreshed so a manual probe made inside
    // the dialog is reflected without remounting the page.
    await wrapper.findAll(".dialog-actions button")[1]!.trigger("click");
    await flushPromises();

    const calls = vi
      .mocked(invoke)
      .mock.calls.filter(
        (call: [string, ...unknown[]]) => call[0] === "get_update_status",
      );
    expect(calls.length).toBeGreaterThanOrEqual(2);
  });
});
