// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

/** Pure helpers for mapping [`CommitSigStatus`] to display text and domain
 * predicates. Rendering — icons, colours, and the detail banner — lives in
 * `src/components/CommitSigIndicator.vue`, which owns the kind→icon→tone→class
 * mapping so consumers no longer reimplement matching CSS classes. */
import type { CommitSigStatus } from "@/api";

/** A short human label for a status (e.g. `"Unsigned"`). */
export function statusLabel(status: CommitSigStatus): string {
  switch (status.kind) {
    case "verified":
      return "Verified";
    case "untrusted_key":
      return "Untrusted signer";
    case "unsigned":
      return "Unsigned";
    case "bad_signature":
      return "Bad signature";
    case "unsupported_format":
      return `Unsupported (${status.format})`;
    case "unknown":
      return "Unknown";
  }
}

/** Is this a verification problem the user might act on? (mirrors Rust.) */
export function isIssue(status: CommitSigStatus): boolean {
  return status.kind !== "verified";
}

/** Can this be dismissed via an ignore? BadSignature never can. (mirrors Rust.) */
export function isIgnorable(status: CommitSigStatus): boolean {
  return status.kind !== "verified" && status.kind !== "bad_signature";
}

/** The signer fingerprint, when the status carries one. */
export function signerFp(status: CommitSigStatus): string | null {
  return status.kind === "verified" || status.kind === "untrusted_key"
    ? status.signer_fp
    : null;
}
