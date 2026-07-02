// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

/** Helpers for rendering [`CommitSigStatus`] consistently across the badge,
 * the history screen, and the pull modals. */
import type { CommitSigStatus } from "@/types";

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

/** A single glyph for compact display (badge, history row). */
export function statusGlyph(status: CommitSigStatus): string {
  switch (status.kind) {
    case "verified":
      return "✓";
    case "untrusted_key":
      return "⚠";
    case "unsigned":
      return "—";
    case "bad_signature":
      return "⛔";
    case "unsupported_format":
      return "?";
    case "unknown":
      return "?";
  }
}

/** A CSS class suffix for colouring the glyph/row. */
export function statusClass(status: CommitSigStatus): string {
  switch (status.kind) {
    case "verified":
      return "sig-verified";
    case "unsigned":
    case "untrusted_key":
    case "unsupported_format":
    case "unknown":
      return "sig-warn";
    case "bad_signature":
      return "sig-bad";
  }
}

/** A background CSS class for a status banner (success / warn / danger). */
export function statusBgClass(status: CommitSigStatus): string {
  switch (status.kind) {
    case "verified":
      return "bg-success-soft text-success";
    case "bad_signature":
      return "bg-danger-soft text-danger";
    default:
      return "bg-warning-soft text-warning";
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
