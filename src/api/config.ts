// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import type { CommitIdentity, LockMode, RepoConfig } from "./common";

/**
 * Repo-config IPC — mirrors `src-tauri/src/config.rs`. Each setter returns the
 * freshly-persisted {@link RepoConfig} so callers refresh their cached copy from
 * the single authoritative response (no re-fetch).
 */

/** Read the full repository config (URL, auth, lock mode, clear timers, …). */
export async function getConfig(): Promise<RepoConfig> {
  return invoke<RepoConfig>("get_config");
}

/** Read the app's default commit author identity (used as a form hint). */
export async function getCommitIdentityDefault(): Promise<CommitIdentity> {
  return invoke<CommitIdentity>("get_commit_identity_default");
}

/**
 * Persist a custom commit author identity. `null` for either field clears it
 * (the app default applies). Returns the updated config.
 */
export async function setCommitIdentity(
  name: string | null,
  email: string | null,
): Promise<RepoConfig> {
  return invoke<RepoConfig>("set_commit_identity", { name, email });
}

/** Set the app auto-lock mode; returns the updated config. */
export async function setLockMode(mode: LockMode): Promise<RepoConfig> {
  return invoke<RepoConfig>("set_lock_mode", { mode });
}

/** Set the password-view auto-clear seconds (`null` ⇒ default, `0` ⇒ never). */
export async function setViewClearSecs(
  secs: number | null,
): Promise<RepoConfig> {
  return invoke<RepoConfig>("set_view_clear_secs", { secs });
}

/** Set the clipboard auto-clear seconds (`null` ⇒ default, `0` ⇒ never). */
export async function setClipboardClearSecs(
  secs: number | null,
): Promise<RepoConfig> {
  return invoke<RepoConfig>("set_clipboard_clear_secs", { secs });
}

/** Emergency reset: wipe the local store + config and return to setup. */
export async function resetConfig(): Promise<void> {
  await invoke("reset_config");
}
