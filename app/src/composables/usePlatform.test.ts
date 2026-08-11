// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPlatform } from "./usePlatform";

vi.mock("@tauri-apps/api/core");

const fn = () => invoke as ReturnType<typeof vi.fn>;

describe("usePlatform", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("defaults to 'unknown' before init (no platform feature activates)", () => {
    const { platform } = createPlatform();
    expect(platform.value).toBe("unknown");
  });

  it("createPlatform seeds a concrete platform for tests", () => {
    const { platform } = createPlatform({ platform: "android" });
    expect(platform.value).toBe("android");
  });

  it("initPlatform resolves the platform from runtime_platform", async () => {
    fn().mockImplementation((cmd: string) => {
      if (cmd === "runtime_platform") return Promise.resolve("android");
      return Promise.resolve(undefined);
    });
    const { platform, initPlatform } = createPlatform();
    await initPlatform();
    expect(platform.value).toBe("android");
  });

  it("initPlatform leaves 'unknown' on a bridge rejection (fail-open, no feature)", async () => {
    // A rejection means the IPC bridge is broken (the command is a sync cfg!,
    // not a normal flake). Unlike secureAvailable, platform does NOT fail-closed
    // to android — it stays 'unknown', activating no platform-specific feature.
    fn().mockRejectedValue(new Error("bridge"));
    const { platform, initPlatform } = createPlatform();
    await initPlatform();
    expect(platform.value).toBe("unknown");
  });

  it("initPlatform is idempotent (runtime_platform fetched once)", async () => {
    fn().mockResolvedValue("linux");
    const { initPlatform } = createPlatform();
    await initPlatform();
    await initPlatform();
    const calls = fn().mock.calls.filter((c) => c[0] === "runtime_platform");
    expect(calls).toHaveLength(1);
  });
});
