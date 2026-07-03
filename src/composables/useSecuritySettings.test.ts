// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import type { RepoConfig } from "@/api";
import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  createSecuritySettings,
  type SecuritySettingsState,
} from "./useSecuritySettings";

vi.mock("@tauri-apps/api/core");

/** Minimal RepoConfig varying only the view-clear seconds. */
function cfg(view_clear_secs: number | null): RepoConfig {
  return {
    url: "",
    pat: null,
    ssh_key: null,
    ssh_passphrase: null,
    local_path: "",
    view_clear_secs,
  };
}

describe("useSecuritySettings", () => {
  let s: SecuritySettingsState;

  beforeEach(() => {
    vi.clearAllMocks();
    // Fresh per test — no module singleton to reset.
    s = createSecuritySettings();
  });

  it("defaults to a 45s view-clear", () => {
    expect(s.viewClearSecs.value).toBe(45);
  });

  it("loadSecuritySettings applies the backend view_clear_secs", async () => {
    vi.mocked(invoke).mockResolvedValue(cfg(120));
    await s.loadSecuritySettings();
    expect(s.viewClearSecs.value).toBe(120);
    expect(invoke).toHaveBeenCalledWith("get_config");
  });

  it("loadSecuritySettings is idempotent (get_config fetched once)", async () => {
    vi.mocked(invoke).mockResolvedValue(cfg(10));
    await s.loadSecuritySettings();
    await s.loadSecuritySettings();
    expect(
      vi.mocked(invoke).mock.calls.filter((c) => c[0] === "get_config"),
    ).toHaveLength(1);
  });

  it("loadSecuritySettings keeps defaults when get_config rejects", async () => {
    vi.mocked(invoke).mockRejectedValue(new Error("pre-setup"));
    await s.loadSecuritySettings();
    expect(s.viewClearSecs.value).toBe(45);
  });

  it("applySecurityConfig maps null to the default and 0 to Never", () => {
    s.applySecurityConfig(cfg(null));
    expect(s.viewClearSecs.value).toBe(45);
    s.applySecurityConfig(cfg(0));
    expect(s.viewClearSecs.value).toBe(0);
    s.applySecurityConfig(cfg(180));
    expect(s.viewClearSecs.value).toBe(180);
  });
});
