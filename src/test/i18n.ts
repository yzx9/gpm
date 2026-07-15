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
import enSettings from "@/locales/en/settings.json";
import enSetup from "@/locales/en/setup.json";
import enSshKey from "@/locales/en/sshKey.json";
import { createI18n } from "vue-i18n";

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
 * The default-locale bundles that tested components render are inlined here so
 * their `t()` calls resolve in tests (page-bundle keys that no test asserts on
 * still resolve to their key strings). Add a page's `en` bundle when its test
 * asserts on its text.
 *
 * A test that needs a locale-aware mount can override the global via
 * `mount(comp, { global: { plugins: [createTestI18n()] } })`.
 */
export function createTestI18n() {
  return createI18n({
    legacy: false,
    locale: "en",
    fallbackLocale: "en",
    messages: {
      en: {
        common: enCommon,
        about: enAbout,
        entries: enEntries,
        entry: enEntry,
        create: enCreate,
        generate: enGenerate,
        history: enHistory,
        settings: enSettings,
        setup: enSetup,
        sshKey: enSshKey,
        addKey: enAddKey,
      },
    },
  });
}
