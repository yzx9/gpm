// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { describe, expect, it } from "vitest";
import { optionalPassphrase } from "./passphrase";

describe("optionalPassphrase", () => {
  it("maps blank to null", () => {
    expect(optionalPassphrase("")).toBeNull();
    expect(optionalPassphrase("   ")).toBeNull();
    expect(optionalPassphrase("\t\n")).toBeNull();
  });

  it("passes the value through byte-verbatim", () => {
    // A passphrase is an opaque secret — surrounding whitespace included,
    // it must reach the backend exactly as typed.
    expect(optionalPassphrase(" pass ")).toBe(" pass ");
    expect(optionalPassphrase("\nleading")).toBe("\nleading");
    expect(optionalPassphrase("two words")).toBe("two words");
    expect(optionalPassphrase("p")).toBe("p");
  });
});
