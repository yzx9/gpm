// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import enCommon from "@/locales/en/common.json";
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
 * `@/composables`, which reaches `@/api` for the same reason. Only `en/common`
 * is inlined, so `t("common.…")` resolves while page-bundle keys resolve to
 * their key strings (existing assertions don't target those).
 *
 * A test that needs a locale-aware mount can override the global via
 * `mount(comp, { global: { plugins: [createTestI18n()] } })`.
 */
export function createTestI18n() {
  return createI18n({
    legacy: false,
    locale: "en",
    fallbackLocale: "en",
    messages: { en: { common: enCommon } },
  });
}
