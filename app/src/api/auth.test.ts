// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { bumpIdleTimer } from "./auth";

vi.mock("@tauri-apps/api/core");

describe("bumpIdleTimer", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("calls the bump_idle_timer command", async () => {
    (invoke as ReturnType<typeof vi.fn>).mockResolvedValue(undefined);
    await bumpIdleTimer();
    expect(invoke).toHaveBeenCalledWith("bump_idle_timer");
  });

  it("swallows errors (best-effort — a missed bump is no bump)", async () => {
    (invoke as ReturnType<typeof vi.fn>).mockRejectedValue(new Error("boom"));
    await expect(bumpIdleTimer()).resolves.toBeUndefined();
  });
});
