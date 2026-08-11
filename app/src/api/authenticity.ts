// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import type {
  AddedTrustedKey,
  AuthenticityConfig,
  CommitSigInfo,
  CommitSigStatus,
  TrustedGpgKey,
  VerifyMode,
} from "./common";

/**
 * Repository authenticity IPC — mirrors `src-tauri/src/authenticity.rs`:
 * commit-signature verification mode, trusted signing keys, and per-commit issue
 * dismissal. The entry-list badge reads {@link getAuthenticityState}; the
 * SettingsPage authenticity card manages keys/mode; HistoryPage + the entry list
 * resolve audit/enforce blocks by trusting a signer or ignoring an issue.
 */

/** Cached authenticity snapshot for the entry-list indicator badge. */
export interface AuthenticityState {
  mode: VerifyMode;
  head_status: CommitSigStatus;
}

/** Read the cached authenticity state for the entry-list badge. */
export async function getAuthenticityState(
  repoId: string,
): Promise<AuthenticityState> {
  return invoke<AuthenticityState>("get_authenticity_state", { repoId });
}

/** Set the verification mode; returns the effective mode (may refuse Enforce). */
export async function setVerificationMode(
  repoId: string,
  mode: VerifyMode,
): Promise<VerifyMode> {
  return invoke<VerifyMode>("set_verification_mode", { repoId, mode });
}

/** Read the persisted authenticity config (mode, trusted keys, ignored issues). */
export async function getAuthenticityConfig(
  repoId: string,
): Promise<AuthenticityConfig> {
  return invoke<AuthenticityConfig>("get_authenticity_config", { repoId });
}

/** Trust a pasted SSH signing public key with a human-readable `label`. */
export async function addTrustedKey(
  repoId: string,
  publicKey: string,
  label: string,
): Promise<void> {
  await invoke("add_trusted_key", { repoId, publicKey, label });
}

/** Add a trusted signing key from an armored block of EITHER format — the
 * backend detects GPG (`-----BEGIN PGP PUBLIC KEY BLOCK-----`) vs SSH and
 * routes to the right trust store. Returns the typed entry so the caller knows
 * which list to refresh. The paste form calls this; there is no client-side
 * format branching. */
export async function addTrustedSigningKey(
  repoId: string,
  armored: string,
  label: string,
): Promise<AddedTrustedKey> {
  return invoke<AddedTrustedKey>("add_trusted_signing_key", {
    repoId,
    armored,
    label,
  });
}

/** Import a trusted GPG public key from a native-picked file — the primary GPG
 * path on Android, where pasting a multi-line armored block is painful. File
 * bytes stay backend-side. */
export async function importTrustedGpgKeyFile(
  repoId: string,
  label: string,
): Promise<TrustedGpgKey> {
  return invoke<TrustedGpgKey>("import_trusted_gpg_key_file", { repoId, label });
}

/** Remove a trusted signing key by fingerprint. */
export async function removeTrustedKey(
  repoId: string,
  fingerprint: string,
): Promise<void> {
  await invoke("remove_trusted_key", { repoId, fingerprint });
}

/** Remove a trusted GPG key by primary fingerprint. */
export async function removeTrustedGpgKey(
  repoId: string,
  fingerprint: string,
): Promise<void> {
  await invoke("remove_trusted_gpg_key", { repoId, fingerprint });
}

/** Per-key parse warnings for the persisted trusted GPG keys (Settings-only).
 * A trusted key that later fails to re-parse surfaces here instead of silently
 * downgrading its commits to `unverified_signature`. */
export async function getGpgKeyParseWarnings(
  repoId: string,
): Promise<string[]> {
  return invoke<string[]>("get_gpg_key_parse_warnings", { repoId });
}

/** Trust the signer of the current HEAD with a `label`. */
export async function trustHeadSigner(
  repoId: string,
  label: string,
): Promise<void> {
  await invoke("trust_head_signer", { repoId, label });
}

/** Trust the signer of a specific `commit` with a `label`. */
export async function trustCommitSigner(
  repoId: string,
  commit: string,
  label: string,
): Promise<void> {
  await invoke("trust_commit_signer", { repoId, commit, label });
}

/** Dismiss the authenticity issue on a specific `commit`. Returns the commit's
 * updated signature info so the caller can refresh the row in place. */
export async function ignoreCommitIssue(
  repoId: string,
  commit: string,
): Promise<CommitSigInfo> {
  return invoke<CommitSigInfo>("ignore_commit_issue", { repoId, commit });
}

/** One page of commits with their signature status: up to `limit` commits
 * starting at `offset`, plus whether more pages remain. */
export interface CommitPage {
  commits: CommitSigInfo[];
  /** `true` when more pages remain past this slice. */
  has_more: boolean;
}

export async function listCommitSignatures(
  repoId: string,
  offset: number,
  limit: number,
): Promise<CommitPage> {
  return invoke<CommitPage>("list_commit_signatures", { repoId, offset, limit });
}
