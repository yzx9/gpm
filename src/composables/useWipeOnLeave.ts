// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { onBeforeUnmount, onMounted } from "vue";
import { useLockState } from "./useLockState";

/**
 * Wipe sensitive state on the three "leaving" events, so a secret held in a Vue
 * ref is dropped eagerly (defense-in-depth + sooner GC eligibility) rather than
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
 * 3. **Hard identity lock** — `useLockState().onLock`, unless `lock: false`. A
 *    *soft* wipe (Immediate post-op) deliberately does NOT fire `onLock`, so a
 *    revealed secret or a half-typed draft survives it — that exclusion is
 *    `useLockState`'s contract, inherited here unchanged. Pass `lock: false`
 *    for holders with no lock semantic (e.g. setup-flow forms, the unlock UI).
 *
 * `wipe` runs as a bare `window` popstate listener (outside Vue's error
 * capture) and may fire twice in one back navigation (popstate then unmount), so
 * it must be **idempotent and must not throw**: reset refs to their empty value,
 * bump any invalidation tokens, clear timers — safe to call repeatedly.
 *
 * Callback-based (not a refs list): real wipes also bump tokens (`token++`) and
 * call sub-component resets (`pf.value?.reset()`), which a refs API can't
 * express; one callback keeps a single shape.
 *
 * Must be called during a component's `setup()` (uses `onMounted`/
 * `onBeforeUnmount`).
 */
export function useWipeOnLeave(
  wipe: () => void,
  opts: { lock?: boolean } = {},
): void {
  onMounted(() => window.addEventListener("popstate", wipe));
  onBeforeUnmount(() => {
    window.removeEventListener("popstate", wipe);
    wipe();
  });
  if (opts.lock !== false) {
    useLockState().onLock(wipe);
  }
}
