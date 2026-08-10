// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import OverviewSection from "./OverviewSection.vue";

vi.mock("@tauri-apps/api/core");

const RELEASES_URL = "https://github.com/yzx9/gpm/releases/latest";

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

  it("renders no update dot or link when up to date", async () => {
    overrides.get_update_status = {
      available: false,
      unacknowledged: false,
      latest_version: null,
    };
    const wrapper = mountSection();
    await flushPromises();

    expect(wrapper.find(".update-dot").exists()).toBe(false);
    expect(wrapper.find(".update-link").exists()).toBe(false);
  });

  it("renders the dot + a release-page link when an update is available", async () => {
    overrides.get_update_status = {
      available: true,
      unacknowledged: true,
      latest_version: "v0.19.0",
    };
    const wrapper = mountSection();
    await flushPromises();

    expect(wrapper.find(".update-dot").exists()).toBe(true);
    const link = wrapper.find(".update-link");
    expect(link.exists()).toBe(true);
    expect(link.attributes("href")).toBe(RELEASES_URL);
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
    expect(wrapper.find(".update-link").exists()).toBe(true);
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

  it("toggles the update-check pref via set_update_check", async () => {
    overrides.get_update_status = {
      available: false,
      unacknowledged: false,
      latest_version: null,
    };
    const wrapper = mountSection();
    await flushPromises();

    // radio[1] is the Off pill (value false) — see BaseOnOffToggle.test.ts.
    await wrapper.findAll('input[type="radio"]')[1]!.trigger("change");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("set_update_check", { enabled: false });
  });
});
