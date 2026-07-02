// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AuthenticityResult } from "./common";

/**
 * Runtime sync IPC — mirrors the sync half of `src-tauri/src/write.rs` plus
 * `git.rs`: pull/push/cancel, divergence resolution, and the `git-progress`
 * event. (Setup-path `clone_repo` lives in `setup.ts`.) A pull either
 * fast-forwards or surfaces a divergence for the user to resolve.
 */

/** Result of a fast-forward pull. */
export interface PullResult {
  changed: boolean;
  head: string;
  authenticity: AuthenticityResult;
}

/** Local-vs-remote divergence preview (no secrets — names/paths only). */
export interface SyncDivergence {
  local_ahead: number;
  remote_ahead: number;
  /** Full hash of the reviewed remote tip; passed back to resolveSyncDivergence. */
  remote_tip: string;
  /** Secret entries (`.age` stripped) present locally, absent remotely — deleted by adopt. */
  local_only_entries: string[];
  /** Secret entries present on both sides whose bytes differ — overwritten by adopt. */
  modified_entries: string[];
  /** Non-secret tracked files changed locally — also discarded/overwritten by a hard reset. */
  other_changed_files: string[];
}

/** Outcome of `pullRepo`: a normal pull, or a divergence to resolve. */
export type SyncOutcome =
  | ({ kind: "fast_forwarded" } & PullResult)
  | ({ kind: "diverged" } & SyncDivergence);

/** Real-time git transfer progress, emitted as the `"git-progress"` event during
 *  clone/pull. Mirrors the Rust `GitProgressEvent` (a subset of `GitProgress`). */
export interface GitProgressEvent {
  total_objects: number;
  received_objects: number;
  received_bytes: number;
  message: string | null;
}

/** Pull the remote. Fast-forwards, or surfaces a divergence to resolve. */
export async function pullRepo(): Promise<SyncOutcome> {
  return invoke<SyncOutcome>("pull_repo");
}

/** Push the local store to the remote. */
export async function pushRepo(): Promise<void> {
  await invoke("push_repo");
}

/** Cancel an in-flight git transfer (clone/pull) via the backend cancel token. */
export async function cancelGit(): Promise<void> {
  await invoke("cancel_git");
}

/** Resolve a divergence by adopting the remote tip (`expectedRemoteOid` from the preview). */
export async function resolveSyncDivergence(
  expectedRemoteOid: string,
): Promise<PullResult> {
  return invoke<PullResult>("resolve_sync_divergence", { expectedRemoteOid });
}

/** Subscribe to git transfer progress during clone/pull; returns an unlisten handle. */
export async function subscribeGitProgress(
  cb: (progress: GitProgressEvent) => void,
): Promise<UnlistenFn> {
  return listen<GitProgressEvent>("git-progress", (e) => cb(e.payload));
}
