// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import {
  filterPackages,
  groupLicenses,
  type LicensePackage,
} from "@/components/about/data";
import { beforeEach, describe, expect, it, vi } from "vitest";

const pkg = (over: Partial<LicensePackage> = {}): LicensePackage => ({
  ecosystem: "rust",
  name: "serde",
  version: "1.0.0",
  license: "MIT OR Apache-2.0",
  repository: "",
  licenseText: "TEXT",
  ...over,
});

describe("groupLicenses", () => {
  it("groups by SPDX expression and counts", () => {
    const groups = groupLicenses([
      pkg({ name: "a", license: "MIT" }),
      pkg({ name: "b", license: "MIT" }),
      pkg({ name: "c", license: "Apache-2.0" }),
    ]);
    expect(groups.map((g) => [g.license, g.count])).toEqual([
      ["MIT", 2],
      ["Apache-2.0", 1],
    ]);
  });

  it("sorts groups by count desc, then license name", () => {
    const groups = groupLicenses([
      pkg({ name: "a", license: "Zlib" }),
      pkg({ name: "b", license: "Apache-2.0" }),
      pkg({ name: "c", license: "Apache-2.0" }),
    ]);
    expect(groups.map((g) => g.license)).toEqual(["Apache-2.0", "Zlib"]);
  });

  it("collapses an empty license to UNKNOWN", () => {
    const groups = groupLicenses([pkg({ name: "a", license: "" })]);
    expect(groups[0].license).toBe("UNKNOWN");
  });

  it("handles an empty input without throwing", () => {
    expect(groupLicenses([])).toEqual([]);
  });

  it("sub-sorts packages by ecosystem then name then version", () => {
    const groups = groupLicenses([
      pkg({ ecosystem: "npm", name: "zeta", version: "1.0.0", license: "MIT" }),
      pkg({
        ecosystem: "rust",
        name: "beta",
        version: "2.0.0",
        license: "MIT",
      }),
      pkg({
        ecosystem: "rust",
        name: "beta",
        version: "1.0.0",
        license: "MIT",
      }),
      pkg({
        ecosystem: "rust",
        name: "alpha",
        version: "1.0.0",
        license: "MIT",
      }),
    ]);
    expect(
      groups[0].packages.map((p) => `${p.ecosystem}:${p.name}@${p.version}`),
    ).toEqual([
      "npm:zeta@1.0.0",
      "rust:alpha@1.0.0",
      "rust:beta@1.0.0",
      "rust:beta@2.0.0",
    ]);
  });
});

describe("filterPackages", () => {
  const list: LicensePackage[] = [
    pkg({ name: "serde", version: "1.0", license: "MIT OR Apache-2.0" }),
    pkg({ name: "tokio", version: "1.40", license: "MIT" }),
    pkg({ name: "age", version: "0.11", license: "MIT OR Apache-2.0" }),
  ];

  it("returns all packages for an empty query", () => {
    expect(filterPackages(list, "").length).toBe(3);
    expect(filterPackages(list, "   ").length).toBe(3);
  });

  it("returns [] for an empty package list regardless of query", () => {
    expect(filterPackages([], "")).toEqual([]);
    expect(filterPackages([], "serde")).toEqual([]);
  });

  it("matches case-insensitively on name", () => {
    expect(filterPackages(list, "TOKIO").map((p) => p.name)).toEqual(["tokio"]);
  });

  it("matches on license expression", () => {
    const r = filterPackages(list, "apache").map((p) => p.name);
    expect(r.sort()).toEqual(["age", "serde"]);
  });

  it("matches on version", () => {
    expect(filterPackages(list, "0.11").map((p) => p.name)).toEqual(["age"]);
  });

  it("returns empty for a non-matching query", () => {
    expect(filterPackages(list, "nonexistent")).toEqual([]);
  });
});

describe("fetchLicenses", () => {
  const sampleDoc = {
    generatedAt: null,
    complete: true,
    note: "",
    ecosystems: { rust: 1 },
    packages: [
      {
        ecosystem: "rust",
        name: "serde",
        version: "1.0.0",
        license: "MIT OR Apache-2.0",
        repository: "",
        licenseText: "MIT...",
      },
    ],
  };

  beforeEach(() => {
    // The module caches its in-flight promise; reset modules so each test gets
    // a fresh cache.
    vi.resetModules();
  });

  it("parses a successful fetch", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: true,
        json: () => Promise.resolve(sampleDoc),
      }),
    );
    const { fetchLicenses } = await import("@/components/about/data");
    const data = await fetchLicenses();
    expect(data.packages.length).toBe(1);
    expect(data.complete).toBe(true);
  });

  it("caches: a second call does not re-fetch", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: () => Promise.resolve(sampleDoc),
    });
    vi.stubGlobal("fetch", fetchMock);
    const { fetchLicenses } = await import("@/components/about/data");
    await fetchLicenses();
    await fetchLicenses();
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("degrades to an empty doc on HTTP failure and clears the cache", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce({
        ok: false,
        status: 404,
        json: () => Promise.resolve({}),
      })
      .mockResolvedValueOnce({
        ok: true,
        json: () => Promise.resolve(sampleDoc),
      });
    vi.stubGlobal("fetch", fetchMock);
    const { fetchLicenses } = await import("@/components/about/data");
    const failed = await fetchLicenses();
    expect(failed.packages).toEqual([]);
    expect(failed.complete).toBe(false);
    // Cache was cleared on failure → next call re-fetches and succeeds.
    const ok = await fetchLicenses();
    expect(ok.packages.length).toBe(1);
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });
});
