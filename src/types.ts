// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

/** Tauri IPC types — mirrors Rust structs */

export interface Entry {
  path: string;
  name: string;
}

/** One page of entries from the paginated list/search commands. */
export interface EntryPage {
  entries: Entry[];
  /** Total entries matching the query, independent of this page's slice. */
  total: number;
  /** `true` when more pages remain past this slice. */
  has_more: boolean;
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
  authenticity: AuthenticityResult;
}

/** Local-vs-remote divergence preview (no secrets — names/paths only). */
export interface SyncDivergence {
  local_ahead: number;
  remote_ahead: number;
  /** Full hash of the reviewed remote tip; passed back to resolve_sync_divergence. */
  remote_tip: string;
  /** Secret entries (`.age` stripped) present locally, absent remotely — deleted by adopt. */
  local_only_entries: string[];
  /** Secret entries present on both sides whose bytes differ — overwritten by adopt. */
  modified_entries: string[];
  /** Non-secret tracked files changed locally — also discarded/overwritten by a hard reset. */
  other_changed_files: string[];
}

/** Outcome of `pull_repo`: a normal pull, or a divergence to resolve. */
export type SyncOutcome =
  | ({ kind: "fast_forwarded" } & PullResult)
  | ({ kind: "diverged" } & SyncDivergence);

export interface RepoConfig {
  url: string;
  pat: string | null;
  ssh_key: string | null;
  ssh_passphrase: string | null;
  local_path: string;
  /** Git commit author name; null/absent uses the app default. */
  commit_user_name?: string | null;
  /** Git commit author email; null/absent uses the app default. */
  commit_user_email?: string | null;
  /** Repository authenticity config. Absent when Off/empty. */
  authenticity?: AuthenticityConfig;
}

/** Default commit author identity (from `get_commit_identity_default`). */
export interface CommitIdentity {
  name: string;
  email: string;
}

export interface AppError {
  code: string;
  message: string;
}

export interface SshKeyPairResult {
  public_key: string;
  private_key: string;
}

/** A freshly generated age x25519 identity + its public recipient. */
export interface AgeIdentityResult {
  identity: string;
  recipient: string;
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

/** Identity metadata from pick_identity_file — bytes stay backend-side. */
export interface PickedIdentityResult {
  key_type: "x25519" | "ssh_ed25519" | "ssh_rsa" | "post_quantum";
  encrypted: boolean;
  /** Best-effort display name from the picker (null if unknown). */
  filename: string | null;
  /** Derived public key. Present when already usable (unencrypted); null until
   * a passphrase is verified for an encrypted file. */
  recipient: string | null;
}

/** Result of verify_picked_identity — the public key once unlocked. */
export interface VerifiedIdentityResult {
  recipient: string;
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

// ── Repository authenticity ────────────────────────────────────────────────

/** Verification mode (serde `lowercase`). */
export type VerifyMode = "off" | "audit" | "enforce";

/** A commit's verification outcome (serde tagged by `kind`, snake_case). */
export type CommitSigStatus =
  | { kind: "verified"; signer_fp: string }
  | { kind: "untrusted_key"; signer_fp: string }
  | { kind: "unsigned" }
  | { kind: "bad_signature" }
  | { kind: "unsupported_format"; format: string }
  | { kind: "unknown" };

/** A trusted signing public key (public — no secret). */
export interface TrustedKey {
  public_key: string;
  fingerprint: string;
  label: string;
  /** HEAD hash when the key was trusted (provenance). */
  added_at_commit: string;
}

/** A user-dismissed commit issue (scoped per commit + status). */
export interface IgnoredIssue {
  commit: string;
  status: CommitSigStatus;
  ignored_at_commit: string;
}

/** Persisted authenticity config (signing.json). */
export interface AuthenticityConfig {
  mode: VerifyMode;
  trusted_keys: TrustedKey[];
  ignored: IgnoredIssue[];
}

/** A commit's metadata + verification status (history list / detail). */
export interface CommitSigInfo {
  hash: string;
  short_hash: string;
  author: string;
  date: string;
  subject: string;
  status: CommitSigStatus;
  ignored: boolean;
}

/** Authenticity outcome of a pull (Audit issues / Enforce block). */
export interface AuthenticityResult {
  mode: VerifyMode;
  new_commits: CommitSigInfo[];
  open_issues: CommitSigInfo[];
  blocked: boolean;
}

/** Cached snapshot for the entry-list indicator badge. */
export interface AuthenticityState {
  mode: VerifyMode;
  head_status: CommitSigStatus;
}

// ── Secret creation (write) ─────────────────────────────────────────────────

/** One input field of a create preset (mirrors `rustpass::template::PresetField`). */
export interface PresetField {
  key: string;
  label: string;
  required: boolean;
}

/** A built-in secret-creation preset (mirrors `rustpass::template::CreatePreset`). */
export interface CreatePreset {
  id: string;
  label: string;
  /** Directory prefix the secret is generated under (e.g. "websites"). */
  prefix: string;
  /** Field keys whose values join to form the secret's name under `prefix`. */
  name_from: string[];
  fields: PresetField[];
}

/** A successful write — short hash of the commit that recorded it. */
export interface WriteResult {
  commit: string;
}

/** A write-path conflict on a same-name remote entry. Carries NO plaintext. */
export interface WriteConflict {
  name: string;
  /** Whether the remote version decrypts with our key. */
  remote_decryptable: boolean;
}

/** Outcome of a create attempt (serde tagged by `kind`, snake_case). */
export type WriteOutcome =
  | { kind: "written"; commit: string }
  | { kind: "conflict"; name: string; remote_decryptable: boolean };

/** How the user chose to resolve a `WriteOutcome` conflict (serde snake_case). */
export type ConflictChoice =
  | "keep_mine"
  | "keep_mine_force"
  | "keep_remote"
  | "cancel";
