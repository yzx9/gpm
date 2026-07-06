// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  currentLocale,
  DEFAULT_LOCALE,
  i18n,
  isSupportedLocale,
  loadBundle,
  normalizeSupported,
  reconcileLocaleFromBackend,
  resolveBootLocale,
  setLocale,
} from "./index";

type LocaleGlobals = { __GPM_LOCALE__?: string };
function setInjectedLocale(value: string | undefined): void {
  const g = globalThis as LocaleGlobals;
  if (value === undefined) {
    delete g.__GPM_LOCALE__;
  } else {
    g.__GPM_LOCALE__ = value;
  }
}

describe("normalizeSupported", () => {
  it("collapses Chinese variants to zh-CN", () => {
    expect(normalizeSupported("zh")).toBe("zh-CN");
    expect(normalizeSupported("zh-CN")).toBe("zh-CN");
    expect(normalizeSupported("zh-Hans-CN")).toBe("zh-CN");
    expect(normalizeSupported("zh-TW")).toBe("zh-CN");
  });

  it("collapses English variants to en", () => {
    expect(normalizeSupported("en")).toBe("en");
    expect(normalizeSupported("en-US")).toBe("en");
    expect(normalizeSupported("EN-gb")).toBe("en");
  });

  it("falls back to the default for unsupported / missing / empty", () => {
    expect(normalizeSupported("fr-FR")).toBe(DEFAULT_LOCALE);
    expect(normalizeSupported(undefined)).toBe(DEFAULT_LOCALE);
    expect(normalizeSupported(null)).toBe(DEFAULT_LOCALE);
    expect(normalizeSupported("")).toBe(DEFAULT_LOCALE);
  });
});

describe("isSupportedLocale", () => {
  it("accepts only the shipped locales", () => {
    expect(isSupportedLocale("en")).toBe(true);
    expect(isSupportedLocale("zh-CN")).toBe(true);
    expect(isSupportedLocale("zh-TW")).toBe(false);
    expect(isSupportedLocale("fr")).toBe(false);
  });
});

describe("resolveBootLocale", () => {
  beforeEach(() => setInjectedLocale(undefined));

  it("uses the backend-injected value when present", () => {
    setInjectedLocale("zh-CN");
    expect(resolveBootLocale()).toBe("zh-CN");
  });

  it("normalizes a non-canonical injected tag", () => {
    setInjectedLocale("zh-Hans-CN");
    expect(resolveBootLocale()).toBe("zh-CN");
  });

  it("falls back to the default when nothing is injected (NOT navigator.language)", () => {
    setInjectedLocale(undefined);
    expect(resolveBootLocale()).toBe(DEFAULT_LOCALE);
  });

  it("treats an unsupported injected value as no signal", () => {
    setInjectedLocale("fr");
    expect(resolveBootLocale()).toBe(DEFAULT_LOCALE);
  });
});

describe("reconcileLocaleFromBackend", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
    i18n.global.locale.value = DEFAULT_LOCALE;
  });

  it("switches when the backend resolves a different locale", async () => {
    vi.mocked(invoke).mockResolvedValue(
      "zh-CN" as unknown as Awaited<ReturnType<typeof invoke>>,
    );
    await reconcileLocaleFromBackend();
    expect(currentLocale()).toBe("zh-CN");
  });

  it("is a no-op when the backend matches the boot locale", async () => {
    vi.mocked(invoke).mockResolvedValue(
      "en" as unknown as Awaited<ReturnType<typeof invoke>>,
    );
    await reconcileLocaleFromBackend();
    expect(currentLocale()).toBe("en");
  });

  it("keeps the boot locale when the IPC fails", async () => {
    vi.mocked(invoke).mockRejectedValue(new Error("no backend"));
    await reconcileLocaleFromBackend();
    expect(currentLocale()).toBe(DEFAULT_LOCALE);
  });
});

describe("setLocale", () => {
  beforeEach(() => {
    i18n.global.locale.value = DEFAULT_LOCALE;
  });

  it("changes the active locale and mirrors it to <html lang>", async () => {
    await setLocale("zh-CN");
    expect(currentLocale()).toBe("zh-CN");
    expect(document.documentElement.lang).toBe("zh-CN");
  });
});

describe("loadBundle", () => {
  it("does not throw on a missing bundle (fallbackLocale covers it)", async () => {
    await expect(loadBundle("zh-CN", "no-such-page")).resolves.toBeUndefined();
  });

  it("skips re-importing a namespace already loaded for that locale", async () => {
    // `en/common` is inlined in createI18n, so it's already present — dedup
    // (presence-based) must short-circuit and never touch mergeLocaleMessage.
    const mergeSpy = vi.spyOn(i18n.global, "mergeLocaleMessage");
    mergeSpy.mockClear();
    await loadBundle("en", "common");
    expect(mergeSpy).not.toHaveBeenCalled();
    mergeSpy.mockRestore();
  });
});
