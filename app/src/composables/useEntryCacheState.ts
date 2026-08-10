// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import {
  subscribeEntryCacheWarmed,
  subscribeEntryCacheWiped,
  type UnlistenFn,
} from "@/api";
import { onScopeDispose, ref, type Ref } from "vue";

/**
 * Per-view mirror of the backend entry-view cache (R086).
 *
 * The backend caches one in-view entry's decrypted content for the view window
 * so a single unlock opens the whole detail view (copy → show → copy-2FA) with
 * no re-prompt. This composable mirrors that cache's state via the symmetric
 * `entry-cache-warmed` / `entry-cache-wiped` events, so the detail page can show
 * a single Unlock gate while cold and the full button set once warm.
 *
 * Per-view, NOT a singleton: instantiate in `EntryDetailPage`'s `setup` (each
 * entry view has its own cache lifecycle; the backend wipes on leave/switch, and
 * the frontend fires `wipeEntryCache` from `useWipeOnLeave`). `entryCached`
 * starts `false` (cold — the leave-wipe cleared any prior cache) and flips to
 * `true` on the first warm event (the probe/first action populating the cache).
 *
 * The backend is the single source of truth; both transitions fire here so the
 * frontend reconciles from either side (R086 D9 — symmetric events avoid the
 * divergence where a wipe races a miss-populate).
 */
export function useEntryCacheState(): {
  /** `true` while the backend holds this entry's decrypted content cached. */
  readonly entryCached: Readonly<Ref<boolean>>;
} {
  const entryCached = ref(false);
  let unlistenWarmed: UnlistenFn | null = null;
  let unlistenWiped: UnlistenFn | null = null;

  // Subscribe on setup; cache the unlistens for scope-dispose.
  void subscribeEntryCacheWarmed(() => {
    entryCached.value = true;
  }).then((un) => {
    unlistenWarmed = un;
  });
  void subscribeEntryCacheWiped(() => {
    entryCached.value = false;
  }).then((un) => {
    unlistenWiped = un;
  });

  onScopeDispose(() => {
    unlistenWarmed?.();
    unlistenWiped?.();
  });

  return { entryCached };
}
