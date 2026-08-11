// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import {
  discardDivergence,
  resolveEntryConflict,
  type AppError,
  type EntryConflictChoice,
  type EntryConflictOp,
  type PullResult,
  type SecretParts,
} from "@/api";
import { onBeforeUnmount, ref, type Ref } from "vue";
import { useI18n } from "vue-i18n";
import { useActiveRepo } from "./useActiveRepo";
import { useLockSignals } from "./useLockSignals";
import { isAuthCancelled, useLockState } from "./useLockState";

/** A `WriteOutcome` `entry_conflict` payload (the `kind` tag stripped by the
 *  caller), carried to the modal and back to `resolveEntryConflict`. */
export interface EntryConflictPayload {
  name: string;
  base_oid: string;
  current_oid: string | null;
  remote_tip: string;
  op: EntryConflictOp;
}

/**
 * Per-entry conflict resolution (R026) for the edit/delete flows — the
 * entry-conflict sibling of {@link useDivergence}. Owns the modal state
 * (`conflict`/`resolving`/`conflictError`) and the resolve/cancel logic.
 *
 * `keep_mine` (edit/create) re-encrypts the caller's body, so it is
 * identity-gated (`runWithAuth`); `keep_mine` (delete) and `keep_theirs` need no
 * identity. `PULL_FF_FAILED` (the remote moved again since the user reviewed)
 * routes to `onPullFfFailed`; `AUTH_CANCELLED` is swallowed. The caller decides
 * the aftermath (toast wording + navigation) via `onResolved`/`onPullFfFailed`.
 *
 * `onAnyLock` clears a pending conflict on either lock (and cancels a parked
 * keep-mine resolve). Cancel reuses `discardDivergence` to release the deferred
 * identity-cache wipe the edit save skipped — abandoning the modal must not
 * strand the cached key.
 *
 * Must be called during a component's `setup()` (uses `useLockState`, `useI18n`).
 */
export function useEntryConflict(opts: {
  /** i18n key for the generic "resolve failed" error line. */
  resolveFailedKey: string;
  /** Resolve succeeded — page toasts/navigates (wording/target differ per op + choice). */
  onResolved: (
    result: PullResult,
    choice: EntryConflictChoice,
    op: EntryConflictOp,
  ) => void;
  /** `PULL_FF_FAILED` — the remote moved since the user reviewed; the page recovers. */
  onPullFfFailed: () => void;
  /** Enforce signature verification refused the resolve's re-fetch (an unverified
   *  remote commit): nothing was committed and HEAD is unchanged. The block itself
   *  is correct; this surfaces it instead of `onResolved`'s success toast (mirrors
   *  the save path's `authenticity_blocked`). */
  onAuthenticityBlocked: (result: PullResult) => void;
}): {
  conflict: Ref<EntryConflictPayload | null>;
  resolving: Ref<boolean>;
  conflictError: Ref<string>;
  /** Surface a conflict (caller strips the outcome `kind` tag first). `parts` is
   *  the edited secret parts to re-send on keep-mine edit (null for delete). */
  openConflict: (
    payload: EntryConflictPayload,
    parts: SecretParts | null,
  ) => void;
  resolveConflict: (choice: EntryConflictChoice) => Promise<void>;
  cancelConflict: () => void;
} {
  const { cancelAuth, runWithAuth } = useLockState();
  const { t } = useI18n();
  const activeRepo = useActiveRepo();

  const conflict = ref<EntryConflictPayload | null>(null);
  const resolving = ref(false);
  const conflictError = ref("");
  /** The edited parts captured on open, re-sent on a keep-mine edit resolve. */
  let pendingParts: SecretParts | null = null;

  // A lock during a pending resolve dismisses the modal (mirrors useDivergence)
  // and cancels a parked keep-mine resolve: `resolveConflict` captures `parts`
  // into its frame BEFORE `runWithAuth` parks it, so nulling `pendingParts`
  // alone leaves the plaintext riding the suspended frame through the lock
  // window — and the resolve would resume (and publish) after unlock with the
  // modal already gone. `cancelAuth` rejects the parked caller with
  // AUTH_CANCELLED (swallowed in resolveConflict), dropping the frame for GC.
  // Also drop the captured plaintext — the page wipes its own refs, but this
  // closure holds a second copy that must not survive the lock (secret
  // hygiene).
  useLockSignals().onAnyLock(() => {
    conflict.value = null;
    conflictError.value = "";
    pendingParts = null;
    cancelAuth();
  });
  // The lock signals fire only on lock events — their registries use
  // `onScopeDispose` to unregister the listener, not to invoke it — so they do
  // NOT cover a page unmount. Null the captured plaintext on unmount too,
  // mirroring the page's own `useWipeOnLeave` (the unmount window of the lock
  // fix).
  onBeforeUnmount(() => {
    pendingParts = null;
  });

  // Resolve the repo id ONCE when the conflict opens (mirrors useDivergence):
  // cancel reuses the same promise so a transient config-read failure at
  // cancel time cannot skip the deferred identity-wipe release below.
  let openRepoId: Promise<string> | null = null;

  function openConflict(
    payload: EntryConflictPayload,
    parts: SecretParts | null,
  ) {
    conflict.value = payload;
    pendingParts = parts;
    conflictError.value = "";
    openRepoId = activeRepo.currentId();
  }

  /** Dismiss without resolving. Reuses `discardDivergence` to release the deferred
   *  identity-cache wipe an edit save skipped — no stranded cached key. */
  function cancelConflict() {
    if (!conflict.value) return;
    conflict.value = null;
    conflictError.value = "";
    pendingParts = null;
    if (openRepoId)
      void openRepoId
        .then((repoId) => discardDivergence(repoId))
        .catch(() => {});
  }

  async function resolveConflict(choice: EntryConflictChoice) {
    if (!conflict.value) return;
    const { name, remote_tip, op } = conflict.value;
    // Only a keep-mine edit/create re-sends the edited parts; delete +
    // keep-theirs carry none.
    const parts =
      (op === "edit" || op === "create") && choice === "keep_mine"
        ? pendingParts
        : null;
    resolving.value = true;
    conflictError.value = "";
    try {
      const repoId = await activeRepo.currentId();
      const result =
        choice === "keep_mine" && (op === "edit" || op === "create")
          ? // keep-mine edit/create re-encrypts → identity-gated (the deferred
            // cache is still warm for edit; create prompts if it expired).
            await runWithAuth(() =>
              resolveEntryConflict(repoId, name, parts, remote_tip, op, choice),
            )
          : await resolveEntryConflict(
              repoId,
              name,
              parts,
              remote_tip,
              op,
              choice,
            );
      conflict.value = null;
      pendingParts = null;
      // Enforce may refuse the resolve's re-fetch (an unverified remote commit).
      // The block is correct (nothing committed, HEAD unchanged) — don't toast
      // "saved"; route to the page's blocked handler instead of onResolved's
      // success toast, mirroring the save path's authenticity_blocked branch.
      // (useDivergence has the same gap — a systemic follow-up, not fixed here.)
      if (result.authenticity?.blocked) {
        opts.onAuthenticityBlocked(result);
        return;
      }
      opts.onResolved(result, choice, op);
    } catch (e) {
      if (isAuthCancelled(e)) return;
      const appError = e as AppError;
      if (appError?.code === "PULL_FF_FAILED") {
        // The remote moved again since the user reviewed — drop the modal and let
        // the page recover (recheck from the list), mirroring useDivergence.
        conflict.value = null;
        opts.onPullFfFailed();
      } else {
        conflictError.value = appError?.message || t(opts.resolveFailedKey);
      }
    } finally {
      resolving.value = false;
    }
  }

  return {
    conflict,
    resolving,
    conflictError,
    openConflict,
    resolveConflict,
    cancelConflict,
  };
}
