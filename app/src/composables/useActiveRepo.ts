// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { inject, type InjectionKey } from "vue";

import { getAppConfig } from "@/api";

/**
 * The active repository id — the value threaded onto every repo-scoped IPC call
 * (`listEntries(repoId, …)`, etc.). Multi-repository (R080).
 *
 * **Step 1:** there is exactly one repository. `currentId()` fetches the
 * persisted registry (`AppConfig.repositories` / `last_active`) on each call —
 * deliberately uncached, so an Emergency Reset + re-setup (which mints a fresh
 * id mid-session) can't strand a stale id. One cheap sealed-config IPC per call;
 * callers `await` it before their first repo-scoped call.
 *
 * **Step 2** swaps this for a `computed` over `useRoute().params.repoId` (the
 * `:repoId` URL segment becomes the source of truth); the switcher + add/remove
 * flows change the route. `currentId()` is the stable seam callers keep
 * regardless.
 */
export interface ActiveRepoStore {
  /** Resolve the active repository id. Uncached — one sealed-config IPC per call
   *  (deliberately, so a mid-session Emergency Reset + re-setup can't strand a
   *  stale id; callers may cache locally if a hot path needs it). Rejects if no
   *  repository is configured — callers only run post-setup, so this is a
   *  "should not happen" guard, not a control-flow path. */
  currentId(): Promise<string>;
}

/** Injection key for [`useActiveRepo`]. */
export const ACTIVE_REPO_KEY: InjectionKey<ActiveRepoStore> =
  Symbol("activeRepo");

/** Resolve the active repository id from the persisted registry (`AppConfig`).
 *  The shared resolution behind [`createActiveRepoStore`]'s `currentId()` — also
 *  called directly by pre-shell callers with no inject provider (the foreground
 *  sync factory, and the setup flows' first push, which run before the app
 *  shell's `provide`). Rejects if no repository is configured. */
export async function resolveActiveRepoId(): Promise<string> {
  const cfg = await getAppConfig();
  const ids = cfg.repositories ?? [];
  // last_active wins iff it names a registered repo; else the first; else none.
  const id =
    cfg.last_active && ids.includes(cfg.last_active)
      ? cfg.last_active
      : (ids[0] ?? null);
  if (!id) throw new Error("no repository configured");
  return id;
}

/** Composition-root factory (called once in `main.ts`). */
export function createActiveRepoStore(): ActiveRepoStore {
  return {
    currentId: resolveActiveRepoId,
  };
}

/** Inject the active-repo store. Throws if used outside a component tree that
 *  provides `ACTIVE_REPO_KEY` (i.e. outside the app shell). */
export function useActiveRepo(): ActiveRepoStore {
  const store = inject(ACTIVE_REPO_KEY);
  if (!store)
    throw new Error(
      "useActiveRepo() called without an ActiveRepoStore provider",
    );
  return store;
}
