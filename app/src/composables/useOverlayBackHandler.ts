// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { Z } from "@/zTiers";
import { inject, onBeforeUnmount, watch, type Ref } from "vue";
import {
  BACK_HANDLER_KEY,
  type BackHandlerHandle,
} from "./useBackHandlerRegistry";

/**
 * Take over the Android back button while `shown` is true: each back press
 * calls `onBack` instead of navigating the webview. The handler is pushed into
 * a shared back-handler registry at z tier `z`; a press fires ONLY the
 * highest-z entry (ties → most-recent push), so two stacked overlays no longer
 * both dismiss. See `useBackHandlerRegistry` for the dispatch rule and the lazy
 * single-listener / stale-registration guards (formerly per-instance, now
 * registry-owned).
 *
 * `z` defaults to `Z.overlay` — non-overlay callers (e.g. a setup-flow
 * step-collapse) need not pass one and defer to any real overlay above them;
 * overlay shells pass their tier explicitly. When `shown` is false (or the
 * component unmounts) the entry is popped, so Tauri's default back behavior is
 * left untouched while nothing is up.
 *
 * `z` is read once at push time; it is stable for an overlay's life (every
 * shell caller mounts via `v-if` with a constant tier).
 *
 * Android-only in effect: the registry's listener only emits on Android, so on
 * desktop this is idle.
 *
 * Must be called from a component `setup()` (uses `watch`/`onBeforeUnmount`).
 */
export function useOverlayBackHandler(
  shown: Ref<boolean>,
  onBack: () => void,
  z: number = Z.overlay,
): void {
  // Provided app-wide via BACK_HANDLER_KEY (main.ts in production,
  // mountWithApp / explicit provide in tests) — same shape as useDialog. Throws
  // loudly if a test mounts a back-consuming component without providing it.
  const registry = inject(BACK_HANDLER_KEY);
  if (!registry) {
    throw new Error(
      "useOverlayBackHandler() requires BACK_HANDLER_KEY to be provided",
    );
  }
  let handle: BackHandlerHandle | null = null;

  watch(
    shown,
    (up) => {
      if (up) {
        handle = registry.push(onBack, z);
      } else if (handle) {
        registry.pop(handle);
        handle = null;
      }
    },
    { immediate: true },
  );

  // Release if the component unmounts while still shown (e.g. navigating away
  // mid-overlay) — otherwise the entry leaks across the unmount.
  onBeforeUnmount(() => {
    if (handle) {
      registry.pop(handle);
      handle = null;
    }
  });
}
