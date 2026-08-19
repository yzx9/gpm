// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import type { AttributeView, SensitiveContent } from "@/api";
import { ref, watch } from "vue";
import { type Claimed, useSecureClaim } from "./useSecureClaim";
import { useSecuritySettings } from "./useSecuritySettings";
import { useWipeOnLeave } from "./useWipeOnLeave";

/**
 * Reveal sensitive content (a decrypted secret) under the app's secure-reveal
 * contract: auto-clear after the configured view-clear seconds (Never ⇒ stays
 * until manual hide / lock / unmount), plus the shared `useWipeOnLeave`
 * lifecycle (wipe on browser back, unmount, and either lock — the identity
 * hard lock or the app-gate re-lock).
 *
 * R031 — this composable owns the screen-capture-protection claim for the
 * revealed secret. `reveal()` accepts a {@link Claimed} value — which only
 * `withClaim` can produce (it raises `FLAG_SECURE` before the secret arrives) —
 * so a caller cannot render a secret without first acquiring the claim.
 * `clear()` releases the claim, and `useWipeOnLeave(clear)` ensures it releases
 * on back/lock/unmount too. Used by the entry detail view.
 *
 * The auto-clear duration comes from the shared security-settings cache, so a
 * settings change reschedules an in-flight reveal live. The lock signals fire
 * only on lock edges — a soft wipe (no-cache mode, post-op) deliberately
 * leaves a revealed password on screen.
 *
 * Must be called during a component's `setup()`.
 */
export function useSecretReveal() {
  const { viewClearSecs } = useSecuritySettings();
  const { withClaim, release } = useSecureClaim();

  const password = ref<string | null>(null);
  const notes = ref<string | null>(null);
  const attributes = ref<AttributeView[] | null>(null);
  const revealed = ref(false);
  /** Seconds remaining until the auto-clear fires. Drives the live "auto-clears
   *  in Ns" countdown; `0` while nothing is revealed or when Never is set. */
  const clearsInSecs = ref(0);
  let autoHideTimer: ReturnType<typeof setTimeout> | null = null;
  let countdownTimer: ReturnType<typeof setInterval> | null = null;
  /** Epoch-ms deadline the wipe is scheduled for. The countdown recomputes the
   *  remaining seconds from this on each tick instead of counting ticks down, so
   *  a throttled/skipped interval can't make the label drift. */
  let wipeDeadline = 0;

  /** Wipe any revealed content, cancel the auto-clear timer, and release the
   *  screen-capture claim. Idempotent — safe for the back/unmount/lock double-fire. */
  function clear() {
    password.value = null;
    notes.value = null;
    attributes.value = null;
    revealed.value = false;
    clearsInSecs.value = 0;
    if (autoHideTimer) {
      clearTimeout(autoHideTimer);
      autoHideTimer = null;
    }
    if (countdownTimer) {
      clearInterval(countdownTimer);
      countdownTimer = null;
    }
    release();
  }

  /** (Re)arm the auto-clear timer from the current setting. `0` (Never) arms no
   *  timer — the reveal stays until `clear()` (manual hide, unmount, back, or a
   *  lock). */
  function armAutoClear() {
    if (autoHideTimer) {
      clearTimeout(autoHideTimer);
      autoHideTimer = null;
    }
    if (countdownTimer) {
      clearInterval(countdownTimer);
      countdownTimer = null;
    }
    const secs = viewClearSecs.value;
    if (secs > 0) {
      // Capture the wipe's absolute deadline once, then recompute the remaining
      // seconds from it on each tick. setInterval has no per-second guarantee on
      // a WebView (background/lock throttling, busy main thread), so counting
      // ticks would let the label drift ahead of wall-clock; recomputing against
      // the deadline keeps it truthful however often the tick actually fires.
      wipeDeadline = Date.now() + secs * 1000;
      clearsInSecs.value = secs;
      autoHideTimer = setTimeout(clear, secs * 1000);
      // Clamped at 1 so the label never flashes "0s" — the wipe hides the whole
      // block the moment it fires (even if the wipe itself is throttled late).
      countdownTimer = setInterval(() => {
        clearsInSecs.value = Math.max(
          1,
          Math.ceil((wipeDeadline - Date.now()) / 1000),
        );
      }, 1000);
    } else {
      clearsInSecs.value = 0;
    }
  }

  /** Reveal `content` (already produced under a live claim via `withClaim`),
   *  replacing any prior reveal and (re)starting the auto-clear timer. The
   *  `Claimed` type is the compile-time guarantee the caller acquired
   *  FLAG_SECURE first. */
  function reveal(
    content: Claimed<
      Pick<SensitiveContent, "attributes" | "notes" | "password">
    >,
  ) {
    password.value = content.password;
    notes.value = content.notes;
    attributes.value = content.attributes;
    revealed.value = true;
    armAutoClear();
  }

  // Security lifecycle: wipe on browser back (popstate), unmount, and a hard
  // identity lock. The global unlock modal keeps this component mounted behind
  // the overlay on auto-lock, so unmount alone can't guarantee a wipe — the
  // explicit back + lock triggers close the gap. Soft wipes are excluded by
  // useLockState's onLock contract, so a revealed secret survives a post-op soft
  // wipe. `clear()` also releases the claim, so FLAG_SECURE drops with the secret.
  useWipeOnLeave(clear);

  // Reschedule an in-flight reveal if the view-clear setting changes under it.
  watch(viewClearSecs, () => {
    if (revealed.value) {
      armAutoClear();
    }
  });

  return {
    attributes,
    password,
    notes,
    revealed,
    clearsInSecs,
    reveal,
    clear,
    withClaim,
  };
}
