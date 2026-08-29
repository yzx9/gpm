// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { describe, expect, it } from "vitest";
import { isSshUrl } from "./url";

// Parity with the Rust `is_ssh_url` unit test (store/sync.rs): the frontend
// gate and the backend guard must classify the same set of URLs.
describe("isSshUrl", () => {
  it.each([
    ["ssh://git@host/owner/repo.git", true],
    ["SSH://GIT@HOST/owner/repo.git", true],
    ["git@github.com:owner/repo.git", true],
    ["https://github.com/owner/repo.git", false],
    ["HTTPS://user@host/owner/repo.git", false],
    ["http://host/owner/repo.git", false],
    ["git://git@host/owner/repo.git", false],
    ["rsync://user@host/owner/repo.git", false],
    ["/local/path/repo.git", false],
    ["host:owner/repo.git", false],
    ["", false],
  ])("%s → %s", (url, expected) => {
    expect(isSshUrl(url)).toBe(expected);
  });
});
