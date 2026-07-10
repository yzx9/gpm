// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import {
  discardDivergence,
  resolveSyncDivergence,
  type AppError,
  type DivergenceChoice,
  type PullResult,
  type SyncDivergence,
} from "@/api";
import { ref, type Ref } from "vue";
import { useI18n } from "vue-i18n";
import { isAuthCancelled, useLockState } from "./useLockState";

/**
 * Shared save-context divergence resolution for the write flows (create-preset,
 * create-custom, entry edit, entry delete). Their divergence handling was
 * near-verbatim duplicated across `CreatePage`, `EntryDetailPage` (edit + delete)
 * — this collapses it. The sync-context divergence in `EntryListPage` stays
 * inline (different contract: no deferred identity wipe, re-runs sync on
 * `PULL_FF_FAILED`, enforce may block after resolve) — see the comment there.
 *
 * Owns the modal state (`divergence`/`resolving`/`divergeError`) and the
 * resolve/cancel logic, including the `keep_mine` → identity-gated /
 * `adopt_remote` → fast-forward split, the `AUTH_CANCELLED` swallow, and the
 * `PULL_FF_FAILED` branch. The caller decides the page-specific aftermath
 * (toast wording + where to navigate) via `onResolved` / `onPullFfFailed`.
 *
 * `onLock` clears the divergence payload on a hard lock (it is not a WebView
 * secret, but a pending resolve over a locked page is meaningless). The caller
 * wipes its own draft/plaintext separately (e.g. via `useWipeOnLeave`).
 *
 * Must be called during a component's `setup()` (uses `useLockState`, `useI18n`).
 */
export function useDivergence(opts: {
  /** i18n key for the generic "resolve failed" error line. */
  resolveFailedKey: string;
  /** Call `discardDivergence()` on cancel (default true). Save-context writes
   *  defer an identity-cache wipe that cancel must release; a caller with no
   *  deferred wipe passes false. */
  discardOnCancel?: boolean;
  /** Resolve succeeded — page toasts and navigates (keys/target differ per page). */
  onResolved: (result: PullResult, choice: DivergenceChoice) => void;
  /** `PULL_FF_FAILED` — the remote moved since the user reviewed; page recovers
   *  (typically toast + leave to recheck from the list). */
  onPullFfFailed: () => void;
}): {
  divergence: Ref<SyncDivergence | null>;
  resolving: Ref<boolean>;
  divergeError: Ref<string>;
  /** Surface a divergence payload (caller strips the outcome `kind` tag first). */
  openDivergence: (preview: SyncDivergence) => void;
  resolveDivergence: (choice: DivergenceChoice) => Promise<void>;
  cancelDivergence: () => void;
} {
  const { onLock, runWithAuth } = useLockState();
  const { t } = useI18n();
  const discardOnCancel = opts.discardOnCancel ?? true;

  const divergence = ref<SyncDivergence | null>(null);
  const resolving = ref(false);
  const divergeError = ref("");

  // A hard lock during a pending resolve dismisses the modal. (Soft wipes
  // deliberately don't fire onLock — a mid-resolve UI surviving one is fine.)
  onLock(() => {
    divergence.value = null;
    divergeError.value = "";
  });

  function openDivergence(preview: SyncDivergence) {
    divergence.value = preview;
    divergeError.value = "";
  }

  /** Dismiss the modal without resolving. The local commit stays and publishes
   *  on the next Sync; release the deferred identity wipe if applicable. */
  function cancelDivergence() {
    if (!divergence.value) return;
    divergence.value = null;
    divergeError.value = "";
    if (discardOnCancel) void discardDivergence().catch(() => {});
  }

  async function resolveDivergence(choice: DivergenceChoice) {
    if (!divergence.value) return;
    resolving.value = true;
    divergeError.value = "";
    const expectedRemoteOid = divergence.value.remote_tip;
    try {
      const result: PullResult =
        choice === "keep_mine"
          ? await runWithAuth(() =>
              resolveSyncDivergence(expectedRemoteOid, choice),
            )
          : await resolveSyncDivergence(expectedRemoteOid, choice);
      divergence.value = null;
      opts.onResolved(result, choice);
    } catch (e) {
      if (isAuthCancelled(e)) return;
      const appError = e as AppError;
      if (appError?.code === "PULL_FF_FAILED") {
        divergence.value = null;
        opts.onPullFfFailed();
      } else {
        divergeError.value = appError?.message || t(opts.resolveFailedKey);
      }
    } finally {
      resolving.value = false;
    }
  }

  return {
    divergence,
    resolving,
    divergeError,
    openDivergence,
    resolveDivergence,
    cancelDivergence,
  };
}
