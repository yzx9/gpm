// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AuthState } from "./common";

/**
 * Session-auth IPC. These two commands straddle backend modules —
 * `setup::get_auth_state` (the configured/encrypted/unlocked snapshot read at
 * the app gate and by the lock state) and `identity::unlock` (open the identity
 * session with a passphrase) — but the frontend treats them together as "what
 * is the auth state, and how do I advance it."
 */

/**
 * Read the auth snapshot: whether a repo is configured, whether the identity is
 * encrypted, whether the session is currently unlocked, and the identity key
 * type. Cheap; browsing the list needs no identity, so this gates navigation.
 */
export async function getAuthState(): Promise<AuthState> {
  return invoke<AuthState>("get_auth_state");
}

/**
 * Unlock the identity session with the age/SSH `passphrase`. Rejects with an
 * {@link AppError} (`WRONG_PASSPHRASE` etc.) on failure; resolves once the
 * identity cache is populated.
 */
export async function unlock(passphrase: string): Promise<void> {
  await invoke("unlock", { passphrase });
}

/** Payload of the `identity-lock-state` event: the backend's identity-cache
 *  lock snapshot. `locked` = the decrypted identity is NOT cached (the next
 *  identity-needing op will require auth); `soft` = a soft wipe (Immediate
 *  post-op) that emptied the cache without raising the hard-lock overlay. */
export interface IdentityLockState {
  locked: boolean;
  soft: boolean;
}

/** Subscribe to backend identity lock-state transitions — hard locks/unlocks
 *  and Immediate-mode soft wipes. The backend is the single source of truth and
 *  emits on every transition; returns an unlisten handle. */
export async function subscribeIdentityLockState(
  cb: (e: IdentityLockState) => void,
): Promise<UnlistenFn> {
  return listen<IdentityLockState>("identity-lock-state", (e) => cb(e.payload));
}
