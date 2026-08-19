// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { watch } from "vue";
import { useI18n } from "vue-i18n";
import { useAppLockState } from "./useAppLockState";
import { useDraftsNotice } from "./useDraftsNotice";
import { useLockState } from "./useLockState";
import { useToast } from "./useToast";

/**
 * Post-unlock "your unsaved changes were cleared" toast. A lock wipe that
 * cleared user-authored content marked the drafts notice (`useWipeOnLeave`
 * → `mark`); at the FIRST unlock edge that follows — the app gate's or the
 * identity's, whichever lands first — consume the notice and toast once.
 * Without it, an editor cleared by a re-lock reappears empty after unlock
 * with no explanation, reading as a bug rather than the lock doing its job.
 *
 * Extracted from `App.vue` (which calls it once in setup) for testability.
 *
 * Must be called during a component's `setup()` (uses watch + inject).
 */
export function useDraftsClearedToast(): void {
  const { t } = useI18n();
  const notice = useDraftsNotice();
  const { toast } = useToast();
  const { appLocked } = useAppLockState();
  const { locked } = useLockState();

  // Shared edge handler: read-and-reset guarantees one toast per lock cycle
  // even when both locks unlock in the same cycle (the second consume is
  // false). Sticky with a × button — the toast lands exactly when the user is
  // busy with the unlock prompt, so a 3s transient would die unread.
  function onUnlockEdge(unlocked: boolean, wasLocked: boolean | undefined) {
    if (wasLocked && !unlocked && notice.consume()) {
      toast.info({ message: t("common.appLock.draftsCleared"), timeout: null });
    }
  }

  watch(appLocked, onUnlockEdge);
  watch(locked, onUnlockEdge);
}
