// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { onBeforeUnmount, onMounted } from "vue";
import { useDraftsNotice } from "./useDraftsNotice";
import { useLockSignals } from "./useLockSignals";

/**
 * Wipe sensitive state on the "leaving" events, so a secret held in a Vue ref
 * is dropped eagerly (defense-in-depth + sooner GC eligibility) rather than
 * left for the garbage collector after the component unmounts. This is the one
 * shared lifecycle every WebView-side secret holder uses; `useSecretReveal`
 * layers its auto-clear timer on top.
 *
 * Triggers:
 * 1. **Browser/Android back** — `window.popstate` fires synchronously during a
 *    back navigation, ahead of the router-driven unmount. Modal back is a
 *    separate Tauri channel (`useOverlayBackHandler` → `onBackButtonPress`), so
 *    this never double-fires with a modal dismiss, and `BaseModalShell` pushes
 *    no history entry — the two coexist.
 * 2. **Component unmount** — `onBeforeUnmount`.
 * 3. **Either lock** — the identity hard lock (`onLock`) or the app-gate
 *    re-lock (`onAppLock`), via `onAnyLock`, unless `lock: false`. The mask a
 *    gate re-lock raises covers the page but does not unmount it — without this
 *    trigger the covered secrets would survive the lock (issue #20). A *soft*
 *    wipe (Immediate post-op) deliberately fires neither signal, so a revealed
 *    secret or a half-typed draft survives it. Pass `lock: false` for holders
 *    with no lock semantic (e.g. setup-flow forms, the unlock UI).
 *
 * `wipe` runs as a bare `window` popstate listener (outside Vue's error
 * capture) and may fire twice in one back navigation (popstate then unmount), so
 * it must be **idempotent and must not throw**: reset refs to their empty value,
 * bump any invalidation tokens, clear timers — safe to call repeatedly. Its
 * return value is ignored on back/unmount; on a lock it may return `true` to
 * mean "cleared user-authored content", which marks the drafts notice for the
 * post-unlock toast (`useDraftsNotice`).
 *
 * Callback-based (not a refs list): real wipes also bump tokens (`token++`) and
 * call sub-component resets (`pf.value?.reset()`), which a refs API can't
 * express; one callback keeps a single shape.
 *
 * Must be called during a component's `setup()` (uses `onMounted`/
 * `onBeforeUnmount`).
 */
export function useWipeOnLeave(
  wipe: () => boolean | void,
  opts: { lock?: boolean } = {},
): void {
  onMounted(() => window.addEventListener("popstate", wipe));
  onBeforeUnmount(() => {
    window.removeEventListener("popstate", wipe);
    wipe();
  });
  if (opts.lock !== false) {
    const notice = useDraftsNotice();
    useLockSignals().onAnyLock(() => {
      if (wipe() === true) {
        notice.mark();
      }
    });
  }
}
