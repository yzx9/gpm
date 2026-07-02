// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { ref, inject, type Ref, type InjectionKey } from "vue";

/**
 * Global toast host — any caller can surface a brief message, including
 * app-shell code that runs before any page mounts (notably the router guard in
 * `main.ts`, which holds the instance directly and can't reach the page-local
 * `showToast` helpers). `App.vue` renders the `toast` ref.
 *
 * Provided app-wide via `TOAST_KEY` (see `main.ts`); tests construct their own
 * via `createToast()` so they never share or reset a module singleton.
 */

/** Reactive toast state consumed by the host (`App.vue`) and driven by `globalToast`. */
export interface ToastState {
  /** Reactive toast message for the host component (`App.vue`) to render. */
  readonly toast: Readonly<Ref<string>>;
  /** Push a transient global toast message (auto-clears after ~3s). */
  globalToast: (message: string) => void;
}

/** Auto-clear window for a global toast, in milliseconds. */
const TOAST_MS = 3000;

/** Injection key for the app-wide toast host. */
export const TOAST_KEY: InjectionKey<ToastState> = Symbol("ToastState");

/**
 * Create a fresh toast host. Production calls this once in `main.ts` and
 * provides it; tests call it per-case for isolation (no module singleton to
 * reset).
 */
export function createToast(): ToastState {
  const toast = ref("");
  let timer: ReturnType<typeof setTimeout> | null = null;

  /** Push a transient global toast message (auto-clears after ~3s). */
  function globalToast(message: string): void {
    toast.value = message;
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
      toast.value = "";
      timer = null;
    }, TOAST_MS);
  }

  return { toast, globalToast };
}

/**
 * Inject the app-wide toast host. Must be called within a component `setup()`
 * under a tree that provided `TOAST_KEY`. Throws if missing so a forgotten
 * `provide` fails loudly.
 */
export function useToast(): ToastState {
  const s = inject(TOAST_KEY);
  if (!s) {
    throw new Error("useToast() requires TOAST_KEY to be provided");
  }
  return s;
}
