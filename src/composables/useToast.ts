// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { inject, ref, type InjectionKey, type Ref } from "vue";

/**
 * Unified toast host — callers surface transient messages via
 * `useToast().toast.success/.danger/.info/.warning(msg | opts)`, or
 * `.show({ msg, variant })` when the variant is decided at runtime. `App.vue`
 * renders the queue once through `ToastHost`; callers never render toasts.
 *
 * Every push returns a `() => void` dismiss fn bound to that toast, so a sticky
 * toast (`timeout: null`) can be closed programmatically without touching ids.
 * The host's × button uses `toast.dismiss(id)`.
 *
 * Provided app-wide via `TOAST_KEY` (see `main.ts`); tests construct their own
 * via `createToast()` so they never share or reset a module singleton.
 */

export type ToastVariant = "success" | "danger" | "info" | "warning";

/** Options accepted by every variant method (and `show`, which adds `variant`). */
export interface ToastOptions {
  /** The message body. */
  message: string;
  /** Auto-close after this many ms. Must be a positive number, or `null` for a
   *  sticky toast (until dismissed or cap-evicted). Non-positive is a caller bug
   *  (instant self-dismiss). Default 3000. */
  timeout?: number | null;
  /** Show an explicit close (×) button. Defaults to `true` when `timeout` is `null`
   *  or > 5000ms, else `false`. A sticky toast needs a dismissal path — leave this
   *  on (default) for a × button, or hold the dismiss fn returned by the push. */
  closable?: boolean;
}

/** `show`-only: pick the variant at runtime. Defaults to `info`. */
export interface ToastShowOptions extends ToastOptions {
  variant?: ToastVariant;
}

/** A single queued toast. */
export interface ToastItem {
  /** Monotonic id scoped to the creating `createToast()` instance. */
  readonly id: number;
  readonly message: string;
  readonly variant: ToastVariant;
  /** Whether the host renders a × button for this item. */
  readonly closable: boolean;
}

/** Variant-scoped push API. Each variant has a plain-`msg` and an `opts` overload. */
export interface ToastApi {
  success(msg: string): () => void;
  success(opts: ToastOptions): () => void;
  danger(msg: string): () => void;
  danger(opts: ToastOptions): () => void;
  info(msg: string): () => void;
  info(opts: ToastOptions): () => void;
  warning(msg: string): () => void;
  warning(opts: ToastOptions): () => void;
  /** Generic push — variant defaults to `info`. */
  show(opts: ToastShowOptions): () => void;
  /** Dismiss by id (the host ×-button path; callers usually use the returned fn). */
  dismiss(id: number): void;
}

/** Reactive toast queue consumed by the host (`ToastHost`) and fed by `toast`. */
export interface ToastState {
  /** Reactive toast queue, oldest first. */
  readonly toasts: Readonly<Ref<readonly ToastItem[]>>;
  /** Push variants + dismiss. */
  readonly toast: ToastApi;
}

/** Default auto-close window, in milliseconds. */
const DEFAULT_TIMEOUT_MS = 3000;
/** A toast with a timeout above this (ms) gets a close button by default. */
const LONG_TIMEOUT_MS = 5000;
/** Max simultaneous toasts; the oldest is dropped while this is exceeded. */
const MAX_TOASTS = 3;

/** Injection key for the app-wide toast host. */
export const TOAST_KEY: InjectionKey<ToastState> = Symbol("ToastState");

/**
 * Create a fresh toast host. Production calls this once in `main.ts` and
 * provides it; tests call it per-case for isolation (no module singleton to
 * reset).
 */
export function createToast(): ToastState {
  const toasts = ref<ToastItem[]>([]);
  let nextId = 0;
  // Per-item timers so dismissing one (cap overflow, manual close) never strands
  // or aliases another's timer.
  const timers = new Map<number, ReturnType<typeof setTimeout>>();

  function dismiss(id: number): void {
    const i = toasts.value.findIndex((t) => t.id === id);
    if (i !== -1) toasts.value.splice(i, 1);
    const tm = timers.get(id);
    if (tm) {
      clearTimeout(tm);
      timers.delete(id);
    }
  }

  /** Push a toast; returns a dismiss fn bound to its id. */
  function push(
    message: string,
    variant: ToastVariant,
    opts: ToastOptions,
  ): () => void {
    const id = nextId++;
    const timeout =
      opts.timeout === undefined ? DEFAULT_TIMEOUT_MS : opts.timeout;
    const closable =
      opts.closable ?? (timeout === null || timeout > LONG_TIMEOUT_MS);
    toasts.value.push({ id, message, variant, closable });
    // Cap: drop oldest (and clear its timer) while over the limit, so a burst
    // can't pile an unbounded stack on a small mobile screen.
    while (toasts.value.length > MAX_TOASTS) {
      const oldest = toasts.value[0];
      toasts.value.shift();
      const tm = timers.get(oldest.id);
      if (tm) {
        clearTimeout(tm);
        timers.delete(oldest.id);
      }
    }
    if (timeout !== null) {
      timers.set(
        id,
        setTimeout(() => dismiss(id), timeout),
      );
    }
    return () => dismiss(id);
  }

  // `success`/`danger`/... — each takes either a plain msg or an opts object.
  function withVariant(
    variant: ToastVariant,
  ): (msgOrOpts: string | ToastOptions) => () => void {
    return (msgOrOpts) => {
      const opts =
        typeof msgOrOpts === "string" ? { message: msgOrOpts } : msgOrOpts;
      return push(opts.message, variant, opts);
    };
  }

  const toast: ToastApi = {
    success: withVariant("success"),
    danger: withVariant("danger"),
    info: withVariant("info"),
    warning: withVariant("warning"),
    show: (opts) => push(opts.message, opts.variant ?? "info", opts),
    dismiss,
  };

  return { toasts, toast };
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
