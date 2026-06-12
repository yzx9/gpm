// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

/** Tauri IPC types — mirrors Rust structs */

export interface Entry {
  path: string;
  name: string;
}

export interface CopyResult {
  success: boolean;
  entry_name: string;
  cleared_after_secs: number;
}

export interface SensitiveContent {
  password: string;
  notes: string;
}

export interface PullResult {
  changed: boolean;
  head: string;
}

export interface RepoConfig {
  url: string;
  pat: string | null;
  ssh_key: string | null;
  ssh_passphrase: string | null;
  local_path: string;
}

export interface AppError {
  code: string;
  message: string;
}

export interface SshKeyPairResult {
  public_key: string;
  private_key: string;
}

export interface SshPublicKeyResult {
  public_key: string;
}

export interface SshPrivateKeyResult {
  private_key: string;
}

export interface RecipientInfo {
  public_key: string;
  comment: string | null;
  key_type: "x25519" | "ssh_ed25519" | "ssh_rsa";
}

/** Auth state snapshot from get_auth_state command. */
export interface AuthState {
  configured: boolean;
  encrypted: boolean;
  unlocked: boolean;
}
