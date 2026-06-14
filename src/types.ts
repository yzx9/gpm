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
  key_type: "x25519" | "ssh_ed25519" | "ssh_rsa" | "post_quantum";
}

/** Auth state snapshot from get_auth_state command. */
export interface AuthState {
  configured: boolean;
  encrypted: boolean;
  unlocked: boolean;
  /** Identity type: "x25519", "ssh_ed25519", "ssh_rsa", "age_encrypted", "post_quantum", "unknown". */
  identity_type: string;
}

/** Identity validation result from validate_identity command. */
export interface IdentityInfoResult {
  key_type: "x25519" | "ssh_ed25519" | "ssh_rsa" | "post_quantum";
  encrypted: boolean;
}

/** Biometric error codes from the Kotlin plugin / Rust app layer. */
export type BiometricErrorCode =
  /** Biometric storage unusable (desktop, Android <11, no biometric enrolled). */
  | "BIOMETRIC_UNAVAILABLE"
  /** User cancelled / chose the negative ("Use passphrase") button. */
  | "BIOMETRIC_CANCELLED"
  /** Keystore key invalidated (new fingerprint enrolled). */
  | "BIOMETRIC_KEY_INVALIDATED"
  /** Too many failed attempts; temporarily locked out. */
  | "BIOMETRIC_LOCKOUT"
  /** Nothing sealed (retrieve called with no stored passphrase). */
  | "BIOMETRIC_NOT_SET"
  /** Catch-all biometric failure. */
  | "BIOMETRIC_FAILED"
  /** Stored passphrase is stale (age path self-heals). */
  | "WRONG_PASSPHRASE";

/** Error from the biometric commands — same `{ code, message }` shape as AppError. */
export interface BiometricError {
  code: BiometricErrorCode | string;
  message: string;
}
