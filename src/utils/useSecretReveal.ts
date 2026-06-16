// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { ref, onMounted, onBeforeUnmount } from "vue";
import type { SensitiveContent } from "../types";
import { onLock } from "./useLockState";

/** How long a revealed secret stays in the DOM before being wiped. */
const AUTO_CLEAR_MS = 30_000;

/**
 * Reveal sensitive content (a decrypted secret) under the app's secure-reveal
 * contract: auto-clear after 30s, wipe on unmount, wipe on browser back
 * navigation, and wipe on identity lock. Shared by the entry detail view and the
 * create-conflict "View existing" path so the lifecycle lives in exactly one
 * place.
 *
 * Must be called during a component's `setup()`.
 */
export function useSecretReveal() {
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

  /** Reveal `content`, replacing any prior reveal and (re)starting the timer. */
  function reveal(content: Pick<SensitiveContent, "password" | "notes">) {
    if (autoHideTimer) {
      clearTimeout(autoHideTimer);
    }
    password.value = content.password;
    notes.value = content.notes;
    revealed.value = true;
    autoHideTimer = setTimeout(clear, AUTO_CLEAR_MS);
  }

  // Security lifecycle: wipe on unmount and on browser back navigation.
  onMounted(() => window.addEventListener("popstate", clear));
  onBeforeUnmount(() => {
    window.removeEventListener("popstate", clear);
    clear();
  });

  // The global unlock modal keeps this component mounted behind the overlay on
  // auto-lock, so an unmount no longer guarantees a wipe — clear explicitly on
  // the lock event too.
  onLock(clear);

  return { password, notes, revealed, reveal, clear };
}
