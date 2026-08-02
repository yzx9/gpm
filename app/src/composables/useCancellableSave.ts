// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { cancelGit } from "@/api";
import { ref } from "vue";

/**
 * Shared "cancel an in-flight save/delete" affordance for the write flows
 * (create-custom, create-preset, entry edit, entry delete). Owns the `cancelling`
 * ref (a cancel request is in flight → the button shows "Cancelling…") and fires
 * `cancel_git`; the save itself surfaces `WriteOutcome::Cancelled` (or an
 * `Err(CANCELLED)`), which each page routes to its own benign toast.
 *
 * The cancel token is armed under `write_mu` by the rustpass orchestrator, so
 * this targets the running op. `cancelling` is reset by the page when its
 * `saving`/`submitting`/`deleting` ref flips false (the save settled).
 */
export function useCancellableSave(): {
  cancelling: ReturnType<typeof ref<boolean>>;
  cancelSave: () => Promise<void>;
} {
  const cancelling = ref(false);

  /** Best-effort: ask the backend to flip the in-flight op's cancel token. */
  async function cancelSave() {
    cancelling.value = true;
    try {
      await cancelGit();
    } catch (e) {
      // best-effort — the save continues if the cancel request itself fails
      console.debug("[cancellable-save] cancel failed", e);
    }
  }

  return { cancelling, cancelSave };
}
