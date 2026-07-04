// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";

/**
 * First-run setup IPC — mirrors `src-tauri/src/setup.rs`: clone or create the
 * store, mint/validate an identity, and complete setup. The secret identity
 * itself is staged in backend state by `generate_identity` / `pick_identity_file`
 * and consumed by `complete_setup_from_file` — it never crosses IPC. Only the
 * public recipient (and metadata) comes back to the WebView.
 *
 * `clone_repo` lives here (not in `repo.ts`) because it is a setup-path command,
 * even though the backend nests it under `setup`; `push_repo`/`pull_repo`/sync
 * are the runtime sync surface in `repo.ts`.
 */

/** Kind of identity to mint for the create flow (mirrors Rust `CreateIdentityKind`). */
export type CreateIdentityKind = "age" | "ssh";

/** A public recipient the store encrypts to (from `list_recipients`). */
export interface RecipientInfo {
  public_key: string;
  comment: string | null;
  key_type: "x25519" | "ssh_ed25519" | "ssh_rsa" | "plugin" | "post_quantum";
}

/** Identity validation result from `validate_identity`. */
export interface IdentityInfoResult {
  key_type: "x25519" | "ssh_ed25519" | "ssh_rsa" | "plugin" | "post_quantum";
  encrypted: boolean;
  /** Derived public recipient (`null` for encrypted SSH awaiting unlock). */
  recipient: string | null;
}

/** Identity metadata from `pick_identity_file` — bytes stay backend-side. */
export interface PickedIdentityResult {
  key_type: "x25519" | "ssh_ed25519" | "ssh_rsa" | "plugin" | "post_quantum";
  encrypted: boolean;
  /** Best-effort display name from the picker (null if unknown). */
  filename: string | null;
  /** Derived public key. Present when already usable (unencrypted); null until
   *  a passphrase is verified for an encrypted file. */
  recipient: string | null;
}

/** Result of `verify_picked_identity` — the public key once unlocked. */
export interface VerifiedIdentityResult {
  recipient: string;
}

/** Whether a store is already configured (gates re-bootstrap on create retry). */
export async function isConfigured(): Promise<boolean> {
  return invoke<boolean>("is_configured");
}

/** Whether the just-cloned repo is ready (recipients present, identity usable). */
export async function isRepoReady(): Promise<boolean> {
  return invoke<boolean>("is_repo_ready");
}

/**
 * Clone the remote into the local store. `pat` is HTTPS-only; `sshKey` /
 * `sshPassphrase` are SSH-only (pass `null` for the unused pair).
 */
export async function cloneRepo(
  repoUrl: string,
  pat: string | null,
  sshKey: string | null,
  sshPassphrase: string | null,
): Promise<void> {
  await invoke("clone_repo", { repoUrl, pat, sshKey, sshPassphrase });
}

/**
 * Mint + stage a new identity; returns its public recipient only. For SSH the
 * `passphrase` encrypts the PEM and must be reused at complete; for age it is
 * ignored (pass `null`).
 */
export async function generateIdentity(
  kind: CreateIdentityKind,
  passphrase: string | null,
): Promise<string> {
  return invoke<string>("generate_identity", { kind, passphrase });
}

/**
 * Bootstrap the local store + seed `.age-recipients` + (if a remote is given)
 * record origin. Does NOT push. Pass `null` for the auth fields when there is no
 * remote.
 */
export async function createStore(
  recipient: string,
  repoUrl: string | null,
  pat: string | null,
  sshKey: string | null,
  sshPassphrase: string | null,
): Promise<void> {
  await invoke("create_store", {
    recipient,
    repoUrl,
    pat,
    sshKey,
    sshPassphrase,
  });
}

/** List the store's public recipients (may be empty for a fresh local store). */
export async function listRecipients(): Promise<RecipientInfo[]> {
  return invoke<RecipientInfo[]>("list_recipients");
}

/** Classify a pasted identity (key type + whether it is passphrase-encrypted). */
export async function validateIdentity(
  identity: string,
): Promise<IdentityInfoResult> {
  return invoke<IdentityInfoResult>("validate_identity", { identity });
}

/** Complete setup from a pasted identity (encrypted at rest with `passphrase`). */
export async function completeSetup(
  identity: string,
  passphrase: string | null,
): Promise<void> {
  await invoke("complete_setup", { identity, passphrase });
}

/** Complete setup from the staged (picked/generated) identity file. */
export async function completeSetupFromFile(
  passphrase: string | null,
): Promise<void> {
  await invoke("complete_setup_from_file", { passphrase });
}

/** Open the native file picker and stage the chosen identity file (bytes stay backend-side). */
export async function pickIdentityFile(): Promise<PickedIdentityResult> {
  return invoke<PickedIdentityResult>("pick_identity_file");
}

/** Verify the staged file's passphrase; on success reveals its public recipient. */
export async function verifyPickedIdentity(
  passphrase: string,
): Promise<VerifiedIdentityResult> {
  return invoke<VerifiedIdentityResult>("verify_picked_identity", {
    passphrase,
  });
}

/** Verify a pasted encrypted SSH identity's passphrase; on success reveals its
 *  public recipient. Stateless (no pending file) — for live match feedback
 *  before "Complete Setup". */
export async function verifyPastedIdentity(
  identity: string,
  passphrase: string,
): Promise<VerifiedIdentityResult> {
  return invoke<VerifiedIdentityResult>("verify_pasted_identity", {
    identity,
    passphrase,
  });
}

/** Drop any staged (picked/generated) identity. Best-effort; callers `.catch`. */
export async function clearPendingIdentity(): Promise<void> {
  await invoke("clear_pending_identity");
}
