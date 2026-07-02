// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { createApp, type App } from "vue";

/**
 * Run a composable inside a throwaway host component's `setup()` — the standard
 * harness for composables that rely on lifecycle hooks or provide/inject (both
 * need an active component instance). Mirrors the Vue testing guide's recipe.
 *
 * Pass a `provide` callback to register injections on the app BEFORE the host
 * mounts and the composable runs, e.g. `(app) => app.provide(LOCK_KEY, lock)`.
 * Returns the composable's result and the app — call `app.unmount()` to fire
 * `onUnmounted`/`onScopeDispose` cleanups.
 *
 * For component tests (rendering a `.vue`, clicking, asserting DOM) use
 * `@vue/test-utils`'s `mount(comp, { global: { provide } })` instead — this
 * helper is for headless composable tests only.
 *
 * @see https://vuejs.org/guide/scaling-up/testing.html#testing-composables
 */
export function withSetup<T>(
  composable: () => T,
  provide?: (app: App) => void,
): [T, App] {
  let result!: T;
  const app = createApp({
    setup() {
      result = composable();
      return () => {};
    },
  });
  provide?.(app);
  app.mount(document.createElement("div"));
  return [result, app];
}
