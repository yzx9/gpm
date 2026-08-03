// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import type { CommitIdentity, RepoConfig } from "./common";

/**
 * Repo-config IPC — mirrors `src-tauri/src/config.rs`. Repo-scoped only (URL,
 * auth, commit identity, authenticity) after the RFC 0038 scope split; the
 * app-scoped behavior prefs live on
 * {@link import("./system").AppConfig} (`api/system.ts`). Each setter returns
 * the freshly-persisted {@link RepoConfig} so callers refresh their cached copy
 * from the single authoritative response (no re-fetch).
 */

/** Read the repository config (URL, auth, commit identity, authenticity). */
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

/**
 * Set (or clear) the HTTPS personal access token. `null` (or a blank string)
 * clears it. Returns the updated config — the PAT is masked for display by the
 * backend (`RepoConfigPublic`), so the full token never reaches the WebView.
 */
export async function setPat(pat: string | null): Promise<RepoConfig> {
  return invoke<RepoConfig>("set_pat", { pat });
}

/** Remove the stored SSH key + passphrase; a stored PAT then becomes active. */
export async function clearSshKey(): Promise<RepoConfig> {
  return invoke<RepoConfig>("clear_ssh_key");
}

/**
 * Validate a PAT against the remote before saving it: a read-only `git fetch`
 * into a throwaway ref (HEAD untouched). Rejects on auth/network failure so the
 * caller can refuse to save a bad token.
 */
export async function verifyGitAuth(pat: string): Promise<void> {
  await invoke("verify_git_auth", { pat });
}

/** Emergency reset: wipe the local store + config and return to setup. */
export async function resetConfig(): Promise<void> {
  await invoke("reset_config");
}
