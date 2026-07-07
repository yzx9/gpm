// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
  isSignatureIgnorable,
  isSignatureIssue,
  signatureSignerFp,
} from "./useCommitSignature";

// The pure predicates are unit-tested directly here. The locale-aware
// `signatureLabel` (returned from the `useCommitSignature` composable, which
// calls `useI18n`) is covered end-to-end by CommitSigIndicator.test.ts, which
// mounts the indicator against the test i18n and asserts the rendered labels.

describe("isSignatureIssue", () => {
  it("is false only for verified", () => {
    expect(isSignatureIssue({ kind: "verified", signer_fp: "fp" })).toBe(false);
    expect(isSignatureIssue({ kind: "untrusted_key", signer_fp: "fp" })).toBe(
      true,
    );
    expect(isSignatureIssue({ kind: "unsigned" })).toBe(true);
    expect(isSignatureIssue({ kind: "bad_signature" })).toBe(true);
    expect(
      isSignatureIssue({ kind: "unsupported_format", format: "x509" }),
    ).toBe(true);
    expect(isSignatureIssue({ kind: "unknown" })).toBe(true);
  });
});

describe("isSignatureIgnorable", () => {
  it("is false for verified and bad_signature, true otherwise (mirrors Rust)", () => {
    expect(isSignatureIgnorable({ kind: "verified", signer_fp: "fp" })).toBe(
      false,
    );
    expect(isSignatureIgnorable({ kind: "bad_signature" })).toBe(false);
    expect(
      isSignatureIgnorable({ kind: "untrusted_key", signer_fp: "fp" }),
    ).toBe(true);
    expect(isSignatureIgnorable({ kind: "unsigned" })).toBe(true);
    expect(
      isSignatureIgnorable({ kind: "unsupported_format", format: "x509" }),
    ).toBe(true);
    expect(isSignatureIgnorable({ kind: "unknown" })).toBe(true);
  });
});

describe("signatureSignerFp", () => {
  it("returns the fingerprint only for verified / untrusted_key / unverified_signature", () => {
    expect(signatureSignerFp({ kind: "verified", signer_fp: "AB:CD" })).toBe(
      "AB:CD",
    );
    expect(
      signatureSignerFp({ kind: "untrusted_key", signer_fp: "EF:12" }),
    ).toBe("EF:12");
    expect(signatureSignerFp({ kind: "unsigned" })).toBeNull();
    expect(signatureSignerFp({ kind: "bad_signature" })).toBeNull();
    expect(
      signatureSignerFp({ kind: "unsupported_format", format: "x509" }),
    ).toBeNull();
    expect(signatureSignerFp({ kind: "unknown" })).toBeNull();
  });
});
