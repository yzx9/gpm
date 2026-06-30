// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { classifyIdentity, isValidIdentity } from "./identity";

describe("classifyIdentity", () => {
  it("classifies a native x25519 identity", () => {
    expect(classifyIdentity("AGE-SECRET-KEY-1TEST123")).toBe("x25519");
  });

  it("classifies an age-plugin identity (e.g. age-plugin-yubikey)", () => {
    expect(classifyIdentity("AGE-PLUGIN-YUBIKEY-1QGZKJQYZL98RLMC67F9PJ")).toBe(
      "plugin",
    );
  });

  it("classifies a generic age-plugin identity", () => {
    expect(classifyIdentity("AGE-PLUGIN-FOO-1ABCD")).toBe("plugin");
  });

  it("does not swallow a plugin identity as x25519", () => {
    expect(
      classifyIdentity("AGE-PLUGIN-YUBIKEY-1QGZKJQYZL98RLMC67F9PJ"),
    ).not.toBe("x25519");
  });

  it("classifies a post-quantum identity", () => {
    expect(
      classifyIdentity("AGE-SECRET-KEY-PQ-1QQQQQQQQQQQQQQQQQQQQQQQQQ"),
    ).toBe("post_quantum");
  });

  it("classifies unknown content", () => {
    expect(classifyIdentity("not-a-key")).toBe("unknown");
  });
});

describe("isValidIdentity", () => {
  it("accepts native x25519", () => {
    expect(isValidIdentity("AGE-SECRET-KEY-1TEST123")).toBe(true);
  });

  it("rejects a plugin identity (decrypt not supported yet)", () => {
    expect(isValidIdentity("AGE-PLUGIN-YUBIKEY-1QGZKJQYZL98RLMC67F9PJ")).toBe(
      false,
    );
  });

  it("rejects unknown content", () => {
    expect(isValidIdentity("not-a-key")).toBe(false);
  });
});
