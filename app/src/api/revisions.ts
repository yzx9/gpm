// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { invoke } from "@tauri-apps/api/core";

import type { ClipboardNotifyText } from "@/i18n/native";
import type { CommitSigInfo } from "./common";
import type { AttachmentMeta, CopyResult } from "./secrets";

/**
 * Secret revision-history IPC — mirrors `src-tauri/src/revisions.rs` (R027).
 * Listing is pure metadata (no identity); viewing/copying reuses the reveal +
 * clipboard paths. A revision the current identity can't decrypt, and one that
 * deleted the entry, surface as distinct `RevisionView` kinds so ciphertext
 * never crosses IPC.
 */

/** One page of revisions for a single secret (newest first). */
export interface RevisionPage {
  commits: CommitSigInfo[];
  /** `true` when more matching revisions remain past this slice. */
  has_more: boolean;
  /** HEAD oid this page walked from — pass back on the next page so a
   *  background sync can't drift the window. */
  base_oid: string;
}

/** The decrypted payload of a revision view (same shape as `SensitiveContent`). */
export interface RevisionDecrypted {
  password: string;
  notes: string;
  has_totp: boolean;
  /** When set, this revision is a binary attachment (notes is empty); the UI
   *  shows the attachment notice instead of the reveal block. */
  attachment: AttachmentMeta | null;
}

/** A revision view outcome, discriminated by `kind`. */
export type RevisionView =
  | ({ kind: "decrypted" } & RevisionDecrypted)
  | { kind: "undecryptable" }
  | { kind: "deleted" };

/** List one page of a secret's revisions. Page 0 omits `baseOid` (walks HEAD and
 *  returns it); later pages pass the prior page's `baseOid`. Pure metadata. */
export async function listRevisions(
  entryPath: string,
  offset: number,
  limit: number,
  baseOid?: string,
): Promise<RevisionPage> {
  return invoke<RevisionPage>("list_revisions", {
    entryPath,
    offset,
    limit,
    baseOid: baseOid ?? null,
  });
}

/** View one revision: decrypt and return it, or report `undecryptable` /
 *  `deleted`. The past value crosses IPC only on the `decrypted` kind. */
export async function showRevision(
  entryPath: string,
  commit: string,
): Promise<RevisionView> {
  return invoke<RevisionView>("show_revision", { entryPath, commit });
}

/** Copy a revision's password straight to the clipboard — the past value never
 *  reaches the WebView. Rejects for `undecryptable`/`deleted` (disable copy for
 *  those states); the clipboard auto-clears after the configured timer. */
export async function copyRevision(
  entryPath: string,
  commit: string,
  notify?: ClipboardNotifyText,
): Promise<CopyResult> {
  return invoke<CopyResult>("copy_revision", {
    entryPath,
    commit,
    notifyText: notify,
  });
}
