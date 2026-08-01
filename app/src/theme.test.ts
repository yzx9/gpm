// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import {
  applyTheme,
  normalizeThemeMode,
  reconcileThemeFromBackend,
} from "@/theme";
import { invoke } from "@tauri-apps/api/core";
import { afterEach, describe, expect, it, vi } from "vitest";

describe("normalizeThemeMode", () => {
  it("passes the pinned modes through", () => {
    expect(normalizeThemeMode("light")).toBe("light");
    expect(normalizeThemeMode("dark")).toBe("dark");
  });

  it("degrades absent or unknown values to system", () => {
    // Absent / null / empty
    expect(normalizeThemeMode(undefined)).toBe("system");
    expect(normalizeThemeMode(null)).toBe("system");
    expect(normalizeThemeMode("")).toBe("system");
    // "system" is never a stored value (the frontend sends null for it), and a
    // hand-edited/garbage value must not poison the UI.
    expect(normalizeThemeMode("system")).toBe("system");
    expect(normalizeThemeMode("auto")).toBe("system");
    expect(normalizeThemeMode("DARK")).toBe("system");
  });
});

describe("applyTheme", () => {
  afterEach(() => {
    delete document.documentElement.dataset.theme;
  });

  it("clears the attribute for system (the CSS media query then governs)", () => {
    document.documentElement.dataset.theme = "dark";
    applyTheme("system");
    expect(document.documentElement.dataset.theme).toBeUndefined();
    expect(document.documentElement.hasAttribute("data-theme")).toBe(false);
  });

  it("pins light and dark via the data-theme attribute", () => {
    applyTheme("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");
    applyTheme("light");
    expect(document.documentElement.dataset.theme).toBe("light");
  });

  it("is idempotent", () => {
    applyTheme("dark");
    applyTheme("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");
  });
});

describe("reconcileThemeFromBackend", () => {
  afterEach(() => {
    vi.mocked(invoke).mockReset();
    delete document.documentElement.dataset.theme;
  });

  it("applies a persisted pinned mode", async () => {
    vi.mocked(invoke).mockResolvedValue({ theme_mode: "dark" });
    await reconcileThemeFromBackend();
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("clears the attribute when the preference is absent (system)", async () => {
    document.documentElement.dataset.theme = "dark";
    vi.mocked(invoke).mockResolvedValue({}); // no theme_mode ⇒ system
    await reconcileThemeFromBackend();
    expect(document.documentElement.dataset.theme).toBeUndefined();
  });

  it("degrades an unsupported on-disk value to system", async () => {
    vi.mocked(invoke).mockResolvedValue({ theme_mode: "purple" });
    await reconcileThemeFromBackend();
    expect(document.documentElement.dataset.theme).toBeUndefined();
  });

  it("swallows a backend failure and keeps the CSS default", async () => {
    vi.mocked(invoke).mockRejectedValue(new Error("boom"));
    await expect(reconcileThemeFromBackend()).resolves.toBeUndefined();
    expect(document.documentElement.dataset.theme).toBeUndefined();
  });
});
