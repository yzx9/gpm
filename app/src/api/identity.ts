// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";

/**
 * Identity-management IPC — mirrors `src-tauri/src/identity.rs` (the SSH-key and
 * passphrase commands; session `unlock` lives in {@link ./auth}). Secrets never
 * reach the WebView: key bytes stay backend-side and only the public/private
 * strings the user explicitly exports are returned.
 */

/** A generated SSH key pair (both halves returned only at generation time). */
export interface SshKeyPairResult {
  public_key: string;
  private_key: string;
}

/** The identity's SSH public key (safe to display/copy). */
export interface SshPublicKeyResult {
  public_key: string;
}

/** The exported SSH private key (sensitive — auto-cleared after use). */
export interface SshPrivateKeyResult {
  private_key: string;
}

/** Set (or replace) the identity passphrase, encrypting the identity at rest. */
export async function setPassphrase(passphrase: string): Promise<void> {
  await invoke("set_passphrase", { passphrase });
}

/** Rotate the identity passphrase (requires the current one). */
export async function changePassphrase(
  oldPassphrase: string,
  newPassphrase: string,
): Promise<void> {
  await invoke("change_passphrase", { oldPassphrase, newPassphrase });
}

/**
 * Generate a fresh SSH key pair for the repo remote. `passphrase` (optional)
 * protects the private key at rest. Returns both halves so the caller can wire
 * the private key into the auth fields and surface the public key.
 */
export async function generateSshKey(
  passphrase: string | null,
): Promise<SshKeyPairResult> {
  return invoke<SshKeyPairResult>("generate_ssh_key", { passphrase });
}

/** Read the identity's SSH public key (for display / copy to the remote). */
export async function getSshPublicKey(): Promise<SshPublicKeyResult> {
  return invoke<SshPublicKeyResult>("get_ssh_public_key");
}

/** Export the SSH private key (sensitive — caller must auto-clear it). */
export async function exportSshPrivateKey(): Promise<SshPrivateKeyResult> {
  return invoke<SshPrivateKeyResult>("export_ssh_private_key");
}
