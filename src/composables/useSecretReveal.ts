// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import type { SensitiveContent } from "@/api";
import { ref, watch } from "vue";
import { useSecuritySettings } from "./useSecuritySettings";
import { useWipeOnLeave } from "./useWipeOnLeave";

/**
 * Reveal sensitive content (a decrypted secret) under the app's secure-reveal
 * contract: auto-clear after the configured view-clear seconds (Never ⇒ stays
 * until manual hide / lock / unmount), plus the shared `useWipeOnLeave`
 * lifecycle (wipe on browser back, unmount, and a _hard_ identity lock). Shared
 * by the entry detail view and the create-conflict "View existing" path.
 *
 * The auto-clear duration comes from the shared security-settings cache, so a
 * settings change reschedules an in-flight reveal live. `onLock` fires only on a
 * hard lock — a soft wipe (no-cache mode, post-op) deliberately leaves a
 * revealed password on screen.
 *
 * Must be called during a component's `setup()`.
 */
export function useSecretReveal() {
  const { viewClearSecs } = useSecuritySettings();

  const password = ref<string | null>(null);
  const notes = ref<string | null>(null);
  const revealed = ref(false);
  let autoHideTimer: ReturnType<typeof setTimeout> | null = null;

  /** Wipe any revealed content and cancel the auto-clear timer. */
  function clear() {
    password.value = null;
    notes.value = null;
    revealed.value = false;
    if (autoHideTimer) {
      clearTimeout(autoHideTimer);
      autoHideTimer = null;
    }
  }

  /** (Re)arm the auto-clear timer from the current setting. `0` (Never) arms no
   *  timer — the reveal stays until `clear()` (manual hide, unmount, back, or a
   *  hard lock). */
  function armAutoClear() {
    if (autoHideTimer) {
      clearTimeout(autoHideTimer);
      autoHideTimer = null;
    }
    const secs = viewClearSecs.value;
    if (secs > 0) {
      autoHideTimer = setTimeout(clear, secs * 1000);
    }
  }

  /** Reveal `content`, replacing any prior reveal and (re)starting the timer. */
  function reveal(content: Pick<SensitiveContent, "password" | "notes">) {
    password.value = content.password;
    notes.value = content.notes;
    revealed.value = true;
    armAutoClear();
  }

  // Security lifecycle: wipe on browser back (popstate), unmount, and a hard
  // identity lock. The global unlock modal keeps this component mounted behind
  // the overlay on auto-lock, so unmount alone can't guarantee a wipe — the
  // explicit back + lock triggers close the gap. Soft wipes are excluded by
  // useLockState's onLock contract, so a revealed secret survives a post-op soft
  // wipe.
  useWipeOnLeave(clear);

  // Reschedule an in-flight reveal if the view-clear setting changes under it.
  watch(viewClearSecs, () => {
    if (revealed.value) {
      armAutoClear();
    }
  });

  return { password, notes, revealed, reveal, clear };
}
