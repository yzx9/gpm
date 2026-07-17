// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import enAbout from "@/locales/en/about.json";
import enAddKey from "@/locales/en/addKey.json";
import enCommon from "@/locales/en/common.json";
import enCreate from "@/locales/en/create.json";
import enEntries from "@/locales/en/entries.json";
import enEntry from "@/locales/en/entry.json";
import enGenerate from "@/locales/en/generate.json";
import enHistory from "@/locales/en/history.json";
import enLog from "@/locales/en/log.json";
import enSecurity from "@/locales/en/security.json";
import enSettings from "@/locales/en/settings.json";
import enSetup from "@/locales/en/setup.json";
import enSshKey from "@/locales/en/sshKey.json";
import { defineComponent, h, type Component } from "vue";
import { createI18n, useI18n } from "vue-i18n";

/**
 * The default-locale bundles that tested components render, inlined so their
 * `t()` calls resolve in tests (page-bundle keys that no test asserts on still
 * resolve to their key strings). Add a page's `en` bundle when its test asserts
 * on its text. Shared by both the global test i18n (below) and the local-scope
 * wrapper ({@link withI18nScope}).
 */
const TEST_MESSAGES = {
  en: {
    common: enCommon,
    about: enAbout,
    entries: enEntries,
    entry: enEntry,
    create: enCreate,
    generate: enGenerate,
    history: enHistory,
    log: enLog,
    security: enSecurity,
    settings: enSettings,
    setup: enSetup,
    sshKey: enSshKey,
    addKey: enAddKey,
  },
};

/**
 * Build a minimal vue-i18n instance for component tests. Installed globally
 * (see `src/test/setup.ts`) so any component calling `useI18n()` resolves
 * against it without each test wiring the plugin.
 *
 * Deliberately does NOT import the real `@/i18n` module (nor anything that
 * reaches `@/api`): doing so would eagerly import `@/api` → `system.ts` →
 * `@tauri-apps/api/app` and cache the simple Tauri mock, stealing the richer
 * per-test mock that the back-button tests (`BaseModalShell`,
 * `useOverlayBackHandler`) install for themselves. That constraint is also why
 * this lives in its own file rather than `appTestUtils.ts` — that module pulls
 * `@/composables`, which reaches `@/api` for the same reason.
 *
 * A test that needs a locale-aware mount can override the global via
 * `mount(comp, { global: { plugins: [createTestI18n()] } })`.
 */
export function createTestI18n() {
  return createI18n({
    legacy: false,
    locale: "en",
    fallbackLocale: "en",
    messages: TEST_MESSAGES,
  });
}

/**
 * Wrap `comp` in a host that establishes a local i18n scope, then bake its
 * props in. Mount the returned component via `mountWithApp` (or `mount`).
 *
 * For components rendered with `<i18n-t>`: that translation component resolves
 * its messages through `useScope: 'parent'`, and emits the dev warning
 * `[intlify] Not found parent scope. use the global scope.` when its host is
 * mounted at the `@vue/test-utils` root (no ancestor provides a composer). The
 * host's `useI18n({ messages })` creates a local composer it provides, so
 * `<i18n-t>` finds its parent scope; the local scope carries {@link
 * TEST_MESSAGES}, so keys still resolve exactly as under the global test i18n.
 *
 * `wrapper.text()` / queries traverse into `comp` as usual — the host only adds
 * the i18n scope around it.
 */
export function withI18nScope<P extends Record<string, unknown>>(
  comp: Component,
  props?: P,
) {
  return defineComponent({
    setup() {
      useI18n({ messages: TEST_MESSAGES });
      return () => h(comp, props);
    },
  });
}
