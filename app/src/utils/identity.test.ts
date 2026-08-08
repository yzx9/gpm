// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { describe, expect, it } from "vitest";

import { classifyIdentity, isValidIdentity } from "./identity";

// R085: the shared input→type spec consumed by BOTH this vitest and the Rust
// `classify_identity` test (which reads the same JSON via include_str!). A
// classifier branch added on one side without the other fails one of the two
// tests — this pins the logic drift ts-rs cannot see.
import classifierCases from "../../../crates/rustpass/contracts/identity-classifier-cases.json";

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

  it("accepts an armored PGP secret key (mirrors Rust validate_identity_format)", () => {
    expect(isValidIdentity("-----BEGIN PGP PRIVATE KEY BLOCK-----")).toBe(true);
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

describe("classifyIdentity (cross-language fixture — R085)", () => {
  for (const c of classifierCases) {
    it(`classifies ${JSON.stringify(c.input)} as ${c.expected}`, () => {
      expect(classifyIdentity(c.input)).toBe(c.expected);
    });
  }
});
