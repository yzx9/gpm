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

/** Emergency reset: wipe the local store + config and return to setup. */
export async function resetConfig(): Promise<void> {
  await invoke("reset_config");
}
