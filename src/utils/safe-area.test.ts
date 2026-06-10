// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect, vi, beforeEach } from "vitest";
import { invoke, addPluginListener } from "@tauri-apps/api/core";
import { applySafeAreaInsets } from "./safe-area";

describe("safe-area", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    document.documentElement.style.removeProperty("--safe-area-inset-top");
    document.documentElement.style.removeProperty("--safe-area-inset-bottom");
  });

  it("sets CSS custom properties from invoke result", async () => {
    (invoke as ReturnType<typeof vi.fn>).mockResolvedValue({
      top: 48,
      bottom: 32,
    });

    await applySafeAreaInsets();

    expect(
      document.documentElement.style.getPropertyValue("--safe-area-inset-top"),
    ).toBe("48px");
    expect(
      document.documentElement.style.getPropertyValue(
        "--safe-area-inset-bottom",
      ),
    ).toBe("32px");
  });

  it("calls invoke with correct plugin command", async () => {
    (invoke as ReturnType<typeof vi.fn>).mockResolvedValue({
      top: 0,
      bottom: 0,
    });

    await applySafeAreaInsets();

    expect(invoke).toHaveBeenCalledWith("plugin:safe-area|get_insets");
  });

  it("registers plugin listener for dynamic updates", async () => {
    (invoke as ReturnType<typeof vi.fn>).mockResolvedValue({
      top: 0,
      bottom: 0,
    });

    await applySafeAreaInsets();

    expect(addPluginListener).toHaveBeenCalledWith(
      "safe-area",
      "safe-area-changed",
      expect.any(Function),
    );
  });

  it("handles desktop fallback gracefully", async () => {
    (invoke as ReturnType<typeof vi.fn>).mockRejectedValue(
      new Error("plugin not found"),
    );

    // Should not throw
    await applySafeAreaInsets();

    // CSS should remain unchanged
    expect(
      document.documentElement.style.getPropertyValue("--safe-area-inset-top"),
    ).toBe("");
    expect(
      document.documentElement.style.getPropertyValue(
        "--safe-area-inset-bottom",
      ),
    ).toBe("");
    // Should NOT register a listener when invoke fails
    expect(addPluginListener).not.toHaveBeenCalled();
  });
});
