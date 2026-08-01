// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

/** Commit-signature display helpers — the locale-aware status label plus the
 * pure predicates and signer-fingerprint extractor that mirror the Rust
 * [`CommitSigStatus`] logic. Centralizing these here keeps the kind→label (and
 * kind→icon→tone, which lives in `CommitSigIndicator.vue`) consistent across
 * every consumer: the entry-list HEAD badge + audit row, the history detail
 * sheet, the indicator, and the authenticity-block modal. */
import type { CommitSigStatus } from "@/api";

import { useI18n } from "vue-i18n";

/** Is a commit-signature status a verification problem the user might act on?
 * Pure — no setup, no i18n. Mirrors the Rust `is_issue`. */
export function isSignatureIssue(status: CommitSigStatus): boolean {
  return status.kind !== "verified";
}

/** Can this status be dismissed via an ignore? `BadSignature` never can. Pure.
 * Mirrors the Rust `is_ignorable`. */
export function isSignatureIgnorable(status: CommitSigStatus): boolean {
  return status.kind !== "verified" && status.kind !== "bad_signature";
}

/** The signer fingerprint a status carries, when it carries one. Pure. */
export function signatureSignerFp(status: CommitSigStatus): string | null {
  return status.kind === "verified" ||
    status.kind === "untrusted_key" ||
    status.kind === "unverified_signature"
    ? status.signer_fp
    : null;
}

/**
 * The locale-aware label for a commit-signature status (e.g. `"Unsigned"`).
 *
 * The label keys live under `common.signature.*` (always loaded) so they
 * resolve wherever a status appears. Call this in `setup`; the returned
 * `signatureLabel` closes over the active locale, so callers render it without
 * threading a translator through every call site.
 */
export function useCommitSignature() {
  const { t } = useI18n();

  function signatureLabel(status: CommitSigStatus): string {
    switch (status.kind) {
      case "verified":
        return t("common.signature.verified");
      case "untrusted_key":
        return t("common.signature.untrustedSigner");
      case "unverified_signature":
        return t("common.signature.unverifiedSignature");
      case "unsigned":
        return t("common.signature.unsigned");
      case "bad_signature":
        return t("common.signature.badSignature");
      case "unsupported_format":
        return t("common.signature.unsupportedFormat", {
          format: status.format,
        });
      case "unknown":
        return t("common.signature.unknown");
    }
  }

  return { signatureLabel };
}
