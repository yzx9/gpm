// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { subscribeBackButton, type PluginListener } from "@/api";
import { onBeforeUnmount, watch, type Ref } from "vue";

/**
 * Take over the Android back button while `shown` is true: each back press calls
 * `onBack` instead of navigating the webview (the default `app.tauri.AppPlugin`
 * behavior routes back to JS once a `back-button` listener exists). When `shown`
 * is false the listener is released, so Tauri's default back behavior (webview
 * `goBack` / exit) is left completely untouched.
 *
 * Why per-instance and not a shared registry: an overlay's `onBack` only ever
 * dismisses/cancels that overlay — it never navigates — so even if two overlays
 * were up at once (only reachable as hard-lock-during-conflict, which is hidden
 * behind the lock), both firing is benign. No module-level state, so no
 * test-reset or singleton-lifecycle hazard.
 *
 * Registration is async (one IPC round-trip), so three stale-listener windows
 * are guarded: toggled off during the await, the component unmounting during the
 * await, and a rapid true→false→true toggle superseding an in-flight
 * registration. In every case the stale `PluginListener` is unregistered so a
 * single back press can never fire `onBack` twice or into a dead overlay.
 *
 * Android-only in effect: `onBackButtonPress` only emits on Android, so on
 * desktop this registers an idle listener that never fires.
 *
 * Must be called from a component `setup()` (uses `watch`/`onBeforeUnmount`).
 */
export function useOverlayBackHandler(shown: Ref<boolean>, onBack: () => void) {
  let listener: PluginListener | null = null;
  // Set on unmount so an in-flight registration knows to drop itself.
  let disposed = false;
  // Bumped on every registration attempt; an in-flight one that resolves with a
  // stale token was superseded (e.g. rapid re-show) and must be dropped.
  let registerToken = 0;

  watch(
    shown,
    async (up) => {
      if (up) {
        const myToken = ++registerToken;
        const l = await subscribeBackButton(() => onBack());
        // Stale if toggled off, unmounted, or superseded by a newer registration.
        if (disposed || !shown.value || myToken !== registerToken) {
          void l.unregister();
        } else {
          listener = l;
        }
      } else if (listener) {
        const l = listener;
        listener = null;
        await l.unregister();
      }
    },
    { immediate: true },
  );

  // Release the listener if the component unmounts while the overlay is still up
  // (e.g. navigating away mid-conflict) — otherwise it leaks across the unmount.
  onBeforeUnmount(() => {
    disposed = true;
    if (listener) {
      const l = listener;
      listener = null;
      void l.unregister();
    }
  });
}
