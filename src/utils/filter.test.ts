// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect } from "vitest";
import { filterEntries } from "./filter";
import type { Entry } from "../types";

const entries: Entry[] = [
  { path: "github.com/user.age", name: "github-token" },
  { path: "email/personal.age", name: "personal-email" },
  { path: "servers/prod.age", name: "prod-server" },
  { path: "WiFi/home.age", name: "home-wifi" },
];

describe("filterEntries", () => {
  it("returns all entries when query is empty", () => {
    expect(filterEntries(entries, "")).toEqual(entries);
  });

  it("matches entry name case-insensitively", () => {
    const result = filterEntries(entries, "GitHub");
    expect(result).toEqual([
      { path: "github.com/user.age", name: "github-token" },
    ]);
  });

  it("matches entry path case-insensitively", () => {
    const result = filterEntries(entries, "email/");
    expect(result).toEqual([
      { path: "email/personal.age", name: "personal-email" },
    ]);
  });

  it("returns empty array when nothing matches", () => {
    expect(filterEntries(entries, "nonexistent")).toEqual([]);
  });

  it("supports partial matches", () => {
    const result = filterEntries(entries, "git");
    expect(result).toEqual([
      { path: "github.com/user.age", name: "github-token" },
    ]);
  });

  it("returns empty array for empty entries with non-empty query", () => {
    expect(filterEntries([], "test")).toEqual([]);
  });

  it("matches across both name and path (deduped)", () => {
    // "home" matches both path "WiFi/home.age" and name "home-wifi"
    const result = filterEntries(entries, "home");
    expect(result).toHaveLength(1);
    expect(result[0].name).toBe("home-wifi");
  });

  it("matches multiple entries", () => {
    const result = filterEntries(entries, "personal");
    // "personal" matches name "personal-email"
    expect(result).toHaveLength(1);
    expect(result[0].name).toBe("personal-email");
  });
});
