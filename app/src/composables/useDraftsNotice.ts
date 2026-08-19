// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { inject, type InjectionKey } from "vue";

/**
 * Draft-loss notice — the bridge from lock-side wipers to the post-unlock
 * toast. A wipe that cleared user-authored content calls `mark()`; the toast
 * composable consumes at the next unlock edge (whichever lock fired).
 *
 * Deliberately its own tiny store, NOT a field on `AppLockStore`: both lock
 * layers' wipers mark it, and the identity path must not write into the
 * app-gate store (the gate/identity layering the codebase keeps strict).
 * `consume` is read-and-reset, so no lock-edge reset is needed — one mark per
 * lock cycle, one toast at the unlock that follows it.
 */
export interface DraftsNoticeState {
  /** Record that a lock wipe cleared user-authored content (idempotent). */
  mark: () => void;
  /** Read-and-reset: `true` once per lock cycle that cleared a draft. */
  consume: () => boolean;
}

/** Injection key for the app-wide drafts notice (provided in `main.ts`). */
export const DRAFTS_NOTICE_KEY: InjectionKey<DraftsNoticeState> =
  Symbol("DraftsNotice");

/** Create a fresh drafts-notice instance (composition root + tests). */
export function createDraftsNotice(): DraftsNoticeState {
  let cleared = false;
  return {
    mark: () => {
      cleared = true;
    },
    consume: () => {
      const was = cleared;
      cleared = false;
      return was;
    },
  };
}

/**
 * Inject the app-wide drafts notice. Must be called under a tree that provided
 * `DRAFTS_NOTICE_KEY` — throws if missing so a forgotten provide fails loudly.
 */
export function useDraftsNotice(): DraftsNoticeState {
  const s = inject(DRAFTS_NOTICE_KEY);
  if (!s) {
    throw new Error(
      "useDraftsNotice() requires DRAFTS_NOTICE_KEY to be provided",
    );
  }
  return s;
}
