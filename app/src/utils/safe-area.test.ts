// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { addPluginListener, invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { applySafeAreaInsets } from "./safe-area";

const INSET_VARS = [
  "--safe-area-inset-top",
  "--safe-area-inset-bottom",
  "--safe-area-inset-left",
  "--safe-area-inset-right",
] as const;

function prop(name: string): string {
  return document.documentElement.style.getPropertyValue(name);
}

describe("safe-area", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    for (const v of INSET_VARS) {
      document.documentElement.style.removeProperty(v);
    }
  });

  it("sets all four CSS custom properties from invoke result", async () => {
    (invoke as ReturnType<typeof vi.fn>).mockResolvedValue({
      top: 48,
      bottom: 32,
      left: 12,
      right: 0,
    });

    await applySafeAreaInsets();

    expect(prop("--safe-area-inset-top")).toBe("48px");
    expect(prop("--safe-area-inset-bottom")).toBe("32px");
    expect(prop("--safe-area-inset-left")).toBe("12px");
    expect(prop("--safe-area-inset-right")).toBe("0px");
  });

  it("calls invoke with correct plugin command", async () => {
    (invoke as ReturnType<typeof vi.fn>).mockResolvedValue({
      top: 0,
      bottom: 0,
      left: 0,
      right: 0,
    });

    await applySafeAreaInsets();

    expect(invoke).toHaveBeenCalledWith("plugin:safe-area|get_insets");
  });

  it("registers plugin listener for dynamic updates", async () => {
    (invoke as ReturnType<typeof vi.fn>).mockResolvedValue({
      top: 0,
      bottom: 0,
      left: 0,
      right: 0,
    });

    await applySafeAreaInsets();

    expect(addPluginListener).toHaveBeenCalledWith(
      "safe-area",
      "safe-area-changed",
      expect.any(Function),
    );
  });

  it("applies left/right from the safe-area-changed event payload", async () => {
    // The plugin emits a 4-field payload on safe-area-changed; the registered
    // callback must carry left/right through to the CSS vars (not just the
    // top/bottom it got from get_insets).
    (invoke as ReturnType<typeof vi.fn>).mockResolvedValue({
      top: 0,
      bottom: 0,
      left: 0,
      right: 0,
    });
    await applySafeAreaInsets();

    const cb = vi.mocked(addPluginListener).mock.calls[0][2] as (
      insets: Record<string, number>,
    ) => void;
    cb({ top: 10, bottom: 20, left: 30, right: 40 });

    expect(prop("--safe-area-inset-left")).toBe("30px");
    expect(prop("--safe-area-inset-right")).toBe("40px");
    expect(prop("--safe-area-inset-top")).toBe("10px");
    expect(prop("--safe-area-inset-bottom")).toBe("20px");
  });

  it("re-applies insets when the viewport changes (rotation/resize)", async () => {
    // Initial boot reports 0 (e.g. the plugin's insets listener hasn't fired yet).
    (invoke as ReturnType<typeof vi.fn>).mockResolvedValue({
      top: 0,
      bottom: 0,
      left: 0,
      right: 0,
    });
    await applySafeAreaInsets();

    // A rotation makes get_insets report the real insets; the resize handler
    // must re-pull and apply them without relying on the plugin's event.
    (invoke as ReturnType<typeof vi.fn>).mockResolvedValue({
      top: 40,
      bottom: 24,
      left: 16,
      right: 0,
    });
    window.dispatchEvent(new Event("resize"));

    await vi.waitFor(() => {
      expect(prop("--safe-area-inset-top")).toBe("40px");
      expect(prop("--safe-area-inset-bottom")).toBe("24px");
      expect(prop("--safe-area-inset-left")).toBe("16px");
    });
  });

  it("handles desktop fallback gracefully", async () => {
    (invoke as ReturnType<typeof vi.fn>).mockRejectedValue(
      new Error("plugin not found"),
    );

    // Should not throw
    await applySafeAreaInsets();

    // CSS should remain unchanged
    for (const v of INSET_VARS) {
      expect(prop(v)).toBe("");
    }
    // Should NOT register a listener when invoke fails
    expect(addPluginListener).not.toHaveBeenCalled();
  });
});
