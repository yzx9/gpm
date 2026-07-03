// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
  isIgnorable,
  isIssue,
  signerFp,
  statusGlyph,
  statusLabel,
} from "./signature";

describe("statusLabel", () => {
  it("labels each kind, interpolating format for unsupported_format", () => {
    expect(statusLabel({ kind: "verified", signer_fp: "fp" })).toBe("Verified");
    expect(statusLabel({ kind: "untrusted_key", signer_fp: "fp" })).toBe(
      "Untrusted signer",
    );
    expect(statusLabel({ kind: "unsigned" })).toBe("Unsigned");
    expect(statusLabel({ kind: "bad_signature" })).toBe("Bad signature");
    expect(statusLabel({ kind: "unsupported_format", format: "x509" })).toBe(
      "Unsupported (x509)",
    );
    expect(statusLabel({ kind: "unknown" })).toBe("Unknown");
  });
});

describe("statusGlyph", () => {
  it("returns a glyph per kind", () => {
    expect(statusGlyph({ kind: "verified", signer_fp: "fp" })).toBe("✓");
    expect(statusGlyph({ kind: "untrusted_key", signer_fp: "fp" })).toBe("⚠");
    expect(statusGlyph({ kind: "unsigned" })).toBe("—");
    expect(statusGlyph({ kind: "bad_signature" })).toBe("⛔");
    expect(statusGlyph({ kind: "unsupported_format", format: "x509" })).toBe(
      "?",
    );
    expect(statusGlyph({ kind: "unknown" })).toBe("?");
  });
});

describe("signerFp", () => {
  it("returns the fingerprint only for verified / untrusted_key", () => {
    expect(signerFp({ kind: "verified", signer_fp: "AB:CD" })).toBe("AB:CD");
    expect(signerFp({ kind: "untrusted_key", signer_fp: "EF:12" })).toBe(
      "EF:12",
    );
    expect(signerFp({ kind: "unsigned" })).toBeNull();
    expect(signerFp({ kind: "bad_signature" })).toBeNull();
    expect(signerFp({ kind: "unsupported_format", format: "x509" })).toBeNull();
    expect(signerFp({ kind: "unknown" })).toBeNull();
  });
});

describe("isIssue", () => {
  it("is false only for verified", () => {
    expect(isIssue({ kind: "verified", signer_fp: "fp" })).toBe(false);
    expect(isIssue({ kind: "untrusted_key", signer_fp: "fp" })).toBe(true);
    expect(isIssue({ kind: "unsigned" })).toBe(true);
    expect(isIssue({ kind: "bad_signature" })).toBe(true);
    expect(isIssue({ kind: "unsupported_format", format: "x509" })).toBe(true);
    expect(isIssue({ kind: "unknown" })).toBe(true);
  });
});

describe("isIgnorable", () => {
  it("is false for verified and bad_signature, true otherwise (mirrors Rust)", () => {
    expect(isIgnorable({ kind: "verified", signer_fp: "fp" })).toBe(false);
    expect(isIgnorable({ kind: "bad_signature" })).toBe(false);
    expect(isIgnorable({ kind: "untrusted_key", signer_fp: "fp" })).toBe(true);
    expect(isIgnorable({ kind: "unsigned" })).toBe(true);
    expect(isIgnorable({ kind: "unsupported_format", format: "x509" })).toBe(
      true,
    );
    expect(isIgnorable({ kind: "unknown" })).toBe(true);
  });
});
