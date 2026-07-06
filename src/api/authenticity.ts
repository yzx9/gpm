// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

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
export async function getAuthenticityState(): Promise<AuthenticityState> {
  return invoke<AuthenticityState>("get_authenticity_state");
}

/** Set the verification mode; returns the effective mode (may refuse Enforce). */
export async function setVerificationMode(
  mode: VerifyMode,
): Promise<VerifyMode> {
  return invoke<VerifyMode>("set_verification_mode", { mode });
}

/** Read the persisted authenticity config (mode, trusted keys, ignored issues). */
export async function getAuthenticityConfig(): Promise<AuthenticityConfig> {
  return invoke<AuthenticityConfig>("get_authenticity_config");
}

/** Trust a pasted SSH signing public key with a human-readable `label`. */
export async function addTrustedKey(
  publicKey: string,
  label: string,
): Promise<void> {
  await invoke("add_trusted_key", { publicKey, label });
}

/** Add a trusted signing key from an armored block of EITHER format — the
 * backend detects GPG (`-----BEGIN PGP PUBLIC KEY BLOCK-----`) vs SSH and
 * routes to the right trust store. Returns the typed entry so the caller knows
 * which list to refresh. The paste form calls this; there is no client-side
 * format branching. */
export async function addTrustedSigningKey(
  armored: string,
  label: string,
): Promise<AddedTrustedKey> {
  return invoke<AddedTrustedKey>("add_trusted_signing_key", { armored, label });
}

/** Import a trusted GPG public key from a native-picked file — the primary GPG
 * path on Android, where pasting a multi-line armored block is painful. File
 * bytes stay backend-side. */
export async function importTrustedGpgKeyFile(
  label: string,
): Promise<TrustedGpgKey> {
  return invoke<TrustedGpgKey>("import_trusted_gpg_key_file", { label });
}

/** Remove a trusted signing key by fingerprint. */
export async function removeTrustedKey(fingerprint: string): Promise<void> {
  await invoke("remove_trusted_key", { fingerprint });
}

/** Remove a trusted GPG key by primary fingerprint. */
export async function removeTrustedGpgKey(fingerprint: string): Promise<void> {
  await invoke("remove_trusted_gpg_key", { fingerprint });
}

/** Per-key parse warnings for the persisted trusted GPG keys (Settings-only).
 * A trusted key that later fails to re-parse surfaces here instead of silently
 * downgrading its commits to `unverified_signature`. */
export async function getGpgKeyParseWarnings(): Promise<string[]> {
  return invoke<string[]>("get_gpg_key_parse_warnings");
}

/** Trust the signer of the current HEAD with a `label`. */
export async function trustHeadSigner(label: string): Promise<void> {
  await invoke("trust_head_signer", { label });
}

/** Trust the signer of a specific `commit` with a `label`. */
export async function trustCommitSigner(
  commit: string,
  label: string,
): Promise<void> {
  await invoke("trust_commit_signer", { commit, label });
}

/** Dismiss the authenticity issue on a specific `commit` for this signer. */
export async function ignoreCommitIssue(commit: string): Promise<void> {
  await invoke("ignore_commit_issue", { commit });
}

/** List recent commits with their signature status (paged by `limit`). */
export async function listCommitSignatures(
  limit: number,
): Promise<CommitSigInfo[]> {
  return invoke<CommitSigInfo[]>("list_commit_signatures", { limit });
}
