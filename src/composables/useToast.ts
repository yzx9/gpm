// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { ref } from "vue";

/**
 * Global toast host — a module-scoped singleton so any caller can surface a
 * brief message, including app-shell code that runs before any page mounts
 * (notably the router guard in `main.ts`, which can't reach the page-local
 * `showToast` helpers). `App.vue` renders the `toast` ref.
 */
const toast = ref("");
let timer: ReturnType<typeof setTimeout> | null = null;
const TOAST_MS = 3000;

/** Push a transient global toast message (auto-clears after ~3s). */
export function globalToast(message: string): void {
  toast.value = message;
  if (timer) clearTimeout(timer);
  timer = setTimeout(() => {
    toast.value = "";
    timer = null;
  }, TOAST_MS);
}

/** Reactive toast message for the host component (`App.vue`) to render. */
export function useToast() {
  return { toast };
}

/** Test-only: reset the module singleton between cases. */
export function __resetToastForTests() {
  if (timer) clearTimeout(timer);
  timer = null;
  toast.value = "";
}
