// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import {
  discardDivergence,
  resolveEntryConflict,
  type AppError,
  type EntryConflictChoice,
  type EntryConflictOp,
  type PullResult,
} from "@/api";
import { onBeforeUnmount, ref, type Ref } from "vue";
import { useI18n } from "vue-i18n";
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
 * `keep_mine` (edit) re-encrypts the caller's edited body, so it is
 * identity-gated (`runWithAuth`); `keep_mine` (delete) and `keep_theirs` need no
 * identity. `PULL_FF_FAILED` (the remote moved again since the user reviewed)
 * routes to `onPullFfFailed`; `AUTH_CANCELLED` is swallowed. The caller decides
 * the aftermath (toast wording + navigation) via `onResolved`/`onPullFfFailed`.
 *
 * `onLock` clears a pending conflict on a hard lock. Cancel reuses
 * `discardDivergence` to release the deferred identity-cache wipe the edit save
 * skipped (F4) — abandoning the modal must not strand the cached key.
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
}): {
  conflict: Ref<EntryConflictPayload | null>;
  resolving: Ref<boolean>;
  conflictError: Ref<string>;
  /** Surface a conflict (caller strips the outcome `kind` tag first). `editBody`
   *  is the edited plaintext to re-send on keep-mine edit (null for delete). */
  openConflict: (
    payload: EntryConflictPayload,
    editBody: string | null,
  ) => void;
  resolveConflict: (choice: EntryConflictChoice) => Promise<void>;
  cancelConflict: () => void;
} {
  const { onLock, runWithAuth } = useLockState();
  const { t } = useI18n();

  const conflict = ref<EntryConflictPayload | null>(null);
  const resolving = ref(false);
  const conflictError = ref("");
  /** The edited body captured on open, re-sent on a keep-mine edit resolve. */
  let pendingBody: string | null = null;

  // A hard lock during a pending resolve dismisses the modal (mirrors useDivergence).
  // Also drop the captured plaintext — the page wipes its own refs, but this closure
  // holds a second copy that must not survive the lock or a route-away unmount
  // (secret hygiene, R026 F1). `onLock` covers the hard-lock event; the unmount hook
  // below covers a navigation away while the modal is open, where the modal's own
  // `cancelConflict` (which also nulls `pendingBody`) never fires.
  onLock(() => {
    conflict.value = null;
    conflictError.value = "";
    pendingBody = null;
  });
  // `onLock` fires only on a hard-lock event — `useLockState.onLock` uses
  // `onScopeDispose` to unregister the listener, not to invoke it — so it does NOT
  // cover a page unmount. Null the captured plaintext on unmount too, mirroring the
  // page's own `useWipeOnLeave` (R026: the unmount window of the F1 lock fix).
  onBeforeUnmount(() => {
    pendingBody = null;
  });

  function openConflict(
    payload: EntryConflictPayload,
    editBody: string | null,
  ) {
    conflict.value = payload;
    pendingBody = editBody;
    conflictError.value = "";
  }

  /** Dismiss without resolving. Reuses `discardDivergence` to release the deferred
   *  identity-cache wipe an edit save skipped (F4) — no stranded cached key. */
  function cancelConflict() {
    if (!conflict.value) return;
    conflict.value = null;
    conflictError.value = "";
    pendingBody = null;
    void discardDivergence().catch(() => {});
  }

  async function resolveConflict(choice: EntryConflictChoice) {
    if (!conflict.value) return;
    const { name, remote_tip, op } = conflict.value;
    // Only a keep-mine edit re-sends plaintext; delete + keep-theirs carry none.
    const content =
      op === "edit" && choice === "keep_mine" ? pendingBody : null;
    resolving.value = true;
    conflictError.value = "";
    try {
      const result =
        choice === "keep_mine" && op === "edit"
          ? // keep-mine edit re-encrypts → identity-gated (the F4-deferred cache is
            // still warm; runWithAuth prompts only if it expired).
            await runWithAuth(() =>
              resolveEntryConflict(name, content, remote_tip, op, choice),
            )
          : await resolveEntryConflict(name, content, remote_tip, op, choice);
      conflict.value = null;
      pendingBody = null;
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
