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

/** Outcome of `pullRepo`/`syncRepo`: a normal pull, or a divergence to resolve. */
export type SyncOutcome =
  | ({ kind: "fast_forwarded" } & PullResult)
  | ({ kind: "diverged" } & SyncDivergence);

/** How the user resolves a `diverged` outcome (serde snake_case). "cancel" is
 *  client-side (the frontend just dismisses the modal), so it is absent here. */
export type DivergenceChoice = "adopt_remote" | "keep_mine";

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

/** Manual sync (pull + push) — the publish path when autosync is off, and the
 *  "reconcile both directions" action behind the Sync button. Returns a
 *  `diverged` outcome (pull-side or push-rejection race) for the resolve modal. */
export async function syncRepo(): Promise<SyncOutcome> {
  return invoke<SyncOutcome>("sync_repo");
}

/** Push the local store to the remote. */
export async function pushRepo(): Promise<void> {
  await invoke("push_repo");
}

/** Cancel an in-flight git transfer (clone/pull) via the backend cancel token. */
export async function cancelGit(): Promise<void> {
  await invoke("cancel_git");
}

/** Resolve a divergence per `choice` against the reviewed remote tip
 *  (`expectedRemoteOid` from the preview). `keep_mine` is identity-gated
 *  backend-side (re-encrypts local-only entries onto the remote tip and pushes). */
export async function resolveSyncDivergence(
  expectedRemoteOid: string,
  choice: DivergenceChoice,
): Promise<PullResult> {
  return invoke<PullResult>("resolve_sync_divergence", {
    expectedRemoteOid,
    choice,
  });
}

/** Abandon a save-triggered divergence without resolving — clears the deferred
 *  Immediate-mode identity wipe (the save kept it alive for a possible keep-mine). */
export async function discardDivergence(): Promise<void> {
  await invoke("discard_divergence");
}

/** Subscribe to git transfer progress during clone/pull; returns an unlisten handle. */
export async function subscribeGitProgress(
  cb: (progress: GitProgressEvent) => void,
): Promise<UnlistenFn> {
  return listen<GitProgressEvent>("git-progress", (e) => cb(e.payload));
}
