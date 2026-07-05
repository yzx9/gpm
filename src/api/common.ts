// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

/**
 * Cross-domain Tauri IPC types — mirrors Rust structs in `rustpass` / `src-tauri`.
 *
 * `common.ts` holds the types that cross module boundaries: the universal error
 * envelope, the config/auth snapshots read across pages, and the authenticity
 * primitives embedded inside {@link RepoConfig} / {@link AuthenticityResult}. It
 * imports nothing from sibling `api/` modules, so every other `api/*.ts` may
 * `import type` from here without forming a cycle.
 */

/** Universal IPC error envelope — `{ code, message }`. Sanitized of secrets backend-side. */
export interface AppError {
  code: string;
  message: string;
}

/** Type-narrow a caught value into an {@link AppError}. */
export function asAppError(e: unknown): AppError {
  return e as AppError;
}

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
  /** App auto-lock mode. Absent ⇒ Immediate (the default). Mirrors the Rust
   * `LockMode` (serde externally-tagged, lowercase). */
  lock_mode?: LockMode;
  /** Password-view auto-clear seconds. Absent/null ⇒ default (45); 0 ⇒ never. */
  view_clear_secs?: number | null;
  /** Clipboard auto-clear seconds. Absent/null ⇒ default (45); 0 ⇒ never. */
  clipboard_clear_secs?: number | null;
  /** Whether the app-launch biometric gate is enabled (absent ⇒ false). When
   * enabled, the at-rest master key is sealed behind a biometric-gated key and
   * the whole store is unreadable until the app is unlocked. */
  biometric_app_lock?: boolean;
  /** Whether a successful app-unlock should also unlock the identity session
   * (absent ⇒ false). Independent of the auto-lock timing presets; only
   * meaningful when `biometric_app_lock` is enabled. */
  unlock_identity_with_app?: boolean;
  /** Repository authenticity config. Absent when Off/empty. */
  authenticity?: AuthenticityConfig;
  /** Per-device autosync: when on (absent ⇒ true), every save pull-write-pushes
   *  automatically; when off, saves stay local until a manual Sync publishes. */
  autosync?: boolean;
}

/** How the app auto-locks the identity cache (mirrors Rust `LockMode`). */
export type LockMode = "immediate" | { idle: number } | "never";

/** Default commit author identity (from `get_commit_identity_default`). */
export interface CommitIdentity {
  name: string;
  email: string;
}

/** Auth state snapshot from get_auth_state command. */
export interface AuthState {
  configured: boolean;
  encrypted: boolean;
  unlocked: boolean;
  /** Identity type: "x25519", "ssh_ed25519", "ssh_rsa", "age_encrypted", "plugin", "post_quantum", "unknown". */
  identity_type: string;
}

// ── Repository authenticity ────────────────────────────────────────────────

/** Verification mode (serde `lowercase`). */
export type VerifyMode = "off" | "audit" | "enforce";

/** A commit's verification outcome (serde tagged by `kind`, snake_case). */
export type CommitSigStatus =
  | { kind: "verified"; signer_fp: string }
  | { kind: "untrusted_key"; signer_fp: string }
  | { kind: "unverified_signature"; signer_fp: string }
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
