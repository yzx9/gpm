// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { inject, ref, type InjectionKey, type Ref } from "vue";

/**
 * Imperative confirm/prompt dialog host — the `confirm()`/`prompt()`-shaped
 * sibling of `useToast`. Callers await `useDialog().dialog.confirm(opts)` and
 * get a boolean back; `App.vue` renders the queue once through `DialogHost`,
 * which composes `BaseModalShell`. Callers never render the dialog themselves.
 *
 * This replaces the WebView's native `window.confirm()` so every dismissable
 * popup the user sees is our own UI (the OS-mandated BiometricPrompt / SAF
 * pickers / system-permission dialogs are the only native surfaces left).
 *
 * Provided app-wide via `DIALOG_KEY` (see `main.ts`); tests construct their own
 * via `createDialog()` so they never share or reset a module singleton — same
 * pattern as `useToast`.
 *
 * Queue-backed (not a single current ref): each push enqueues a request with
 * its own promise resolver, and resolving one never touches another. In
 * practice only one is ever pending (dialogs are user-triggered and awaited),
 * but the queue makes two rapid triggers well-defined instead of lost.
 *
 * Phase 1 ships `confirm` only. `prompt` (text entry) is deferred to Phase 2,
 * which also introduces a stack-based back-handler so a prompt can safely stack
 * over an already-open sheet.
 */

/** Options for a yes/no confirm. */
export interface ConfirmOptions {
  /** The question body. Required. */
  message: string;
  /** Optional heading above the message. */
  title?: string;
  /** Confirm-button label; defaults to `common.button.confirm`. */
  confirmLabel?: string;
  /** Cancel-button label; defaults to `common.button.cancel`. */
  cancelLabel?: string;
  /** Style the confirm button as destructive (filled danger). */
  danger?: boolean;
}

/** The kind of dialog a queued request renders. Phase 2 adds `"prompt"`. */
export type DialogKind = "confirm";

/** One queued dialog. The host calls `resolve()` with the user's choice. */
export interface DialogRequest {
  /** Monotonic id scoped to the creating `createDialog()` instance. */
  readonly id: number;
  readonly kind: DialogKind;
  readonly opts: ConfirmOptions;
  /** Settle this dialog: resolves the awaited promise AND shifts it off the queue. */
  readonly resolve: (value: boolean) => void;
}

/** Variant-scoped push API. */
export interface DialogApi {
  /** Show a yes/no confirm; resolves `true` on confirm, `false` on cancel/backdrop/back. */
  confirm(opts: ConfirmOptions): Promise<boolean>;
}

/** Reactive dialog queue consumed by the host (`DialogHost`) and fed by `dialog`. */
export interface DialogState {
  /** Reactive queue, oldest first. */
  readonly pending: Readonly<Ref<readonly DialogRequest[]>>;
  /** Push API. */
  readonly dialog: DialogApi;
}

/** Injection key for the app-wide dialog host. */
export const DIALOG_KEY: InjectionKey<DialogState> = Symbol("DialogState");

/**
 * Create a fresh dialog host. Production calls this once in `main.ts` and
 * provides it; tests call it per-case for isolation (no module singleton).
 */
export function createDialog(): DialogState {
  const pending = ref<DialogRequest[]>([]);
  let nextId = 0;

  function confirm(opts: ConfirmOptions): Promise<boolean> {
    return new Promise<boolean>((res) => {
      const id = nextId++;
      // `settle` is the request's public `resolve`: it both resolves the
      // awaited promise and removes the request from the queue, so the host
      // only needs to call one method and the two never drift apart.
      const settle = (value: boolean) => {
        pending.value = pending.value.filter((r) => r.id !== id);
        res(value);
      };
      pending.value = [
        ...pending.value,
        { id, kind: "confirm", opts, resolve: settle },
      ];
    });
  }

  const dialog: DialogApi = { confirm };
  return { pending, dialog };
}

/**
 * Inject the app-wide dialog host. Must be called within a component `setup()`
 * under a tree that provided `DIALOG_KEY`. Throws if missing so a forgotten
 * `provide` fails loudly.
 */
export function useDialog(): DialogState {
  const s = inject(DIALOG_KEY);
  if (!s) {
    throw new Error("useDialog() requires DIALOG_KEY to be provided");
  }
  return s;
}
