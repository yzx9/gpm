// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { ref, watch, onMounted, onBeforeUnmount } from "vue";
import type { SensitiveContent } from "../types";
import { useLockState } from "./useLockState";
import { useSecuritySettings } from "./useSecuritySettings";

/**
 * Reveal sensitive content (a decrypted secret) under the app's secure-reveal
 * contract: auto-clear after the configured view-clear seconds (Never ⇒ stays
 * until manual hide / lock / unmount), wipe on unmount, wipe on browser back
 * navigation, and wipe on a _hard_ identity lock. Shared by the entry detail
 * view and the create-conflict "View existing" path so the lifecycle lives in
 * exactly one place.
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
  const { onLock } = useLockState();

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

  // Security lifecycle: wipe on unmount and on browser back navigation.
  onMounted(() => window.addEventListener("popstate", clear));
  onBeforeUnmount(() => {
    window.removeEventListener("popstate", clear);
    clear();
  });

  // Reschedule an in-flight reveal if the view-clear setting changes under it.
  watch(viewClearSecs, () => {
    if (revealed.value) {
      armAutoClear();
    }
  });

  // The global unlock modal keeps this component mounted behind the overlay on
  // auto-lock, so an unmount no longer guarantees a wipe — clear explicitly on
  // the lock event too. (Hard lock only — soft wipe must not clear the reveal.)
  onLock(clear);

  return { password, notes, revealed, reveal, clear };
}
