// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { resolvedLocale as resolvedLocaleApi } from "@/api";
import enCommon from "@/locales/en/common.json";
import { createI18n } from "vue-i18n";

/**
 * WebView internationalization.
 *
 * Two paths feed the active locale, so the app renders in the right language
 * on the first frame wherever possible and is eventually correct everywhere:
 *
 *  1. Best-effort inject — the backend bakes the system-locale resolution into
 *     `window.__GPM_LOCALE__` before the page's scripts run (Tauri init script).
 *     This gives a zero-flash first frame for the common case (user tracking the
 *     system language), because the backend's own resolution also starts from
 *     the system locale.
 *  2. Authoritative reconcile — after mount, the frontend asks the backend for
 *     the resolved locale (`resolved_locale` IPC) and corrects if it differs.
 *     This is the only path that can honor a pinned preference, since the init
 *     script can only carry the system locale (app.json isn't readable at Tauri
 *     Builder time on Android). On a pinned-preference device the first frame
 *     is the system-locale guess and the reconcile switches within one frame.
 *
 * The injected value is always the literal `"en"` or `"zh-CN"` (the backend
 * normalizes before injecting), so reading it needs no sanitization — only the
 * normalization below to defend against a future/stale injector.
 */

/** Locales the app ships translations for. */
export const SUPPORTED_LOCALES = ["en", "zh-CN"] as const;
export type SupportedLocale = (typeof SUPPORTED_LOCALES)[number];

/** Locale used when the system locale is neither English nor Chinese. */
export const DEFAULT_LOCALE: SupportedLocale = "en";

/**
 * Message schema — only the inlined default-locale `common` bundle. Page bundles
 * load lazily (to keep the initial payload small, per the RFC), so vue-i18n's
 * typing can't type-check their keys without inlining them — a deliberate
 * trade-off: `common` keys are compile-checked, `t("settings.…")` etc. resolve at
 * runtime against the loaded page bundle with `fallbackLocale` covering misses.
 */
type MessageSchema = { common: typeof enCommon };

/** True if `code` is one of {@link SUPPORTED_LOCALES}. */
export function isSupportedLocale(code: string): code is SupportedLocale {
  return (SUPPORTED_LOCALES as readonly string[]).includes(code);
}

/**
 * Map a BCP-47 tag to a supported locale. Chinese variants → `zh-CN`, English
 * variants → `en`, anything else (or missing) → {@link DEFAULT_LOCALE}. Mirrors
 * the Rust `normalize_system_locale` — keep them in sync.
 */
export function normalizeSupported(
  tag: string | undefined | null,
): SupportedLocale {
  if (!tag) return DEFAULT_LOCALE;
  const lower = tag.toLowerCase();
  if (lower.startsWith("zh")) return "zh-CN";
  if (lower.startsWith("en")) return "en";
  return DEFAULT_LOCALE;
}

interface GpmLocaleGlobals {
  __GPM_LOCALE__?: string;
}

/**
 * Resolve the boot locale for the synchronous first paint: the backend-injected
 * value when present (the fast path), otherwise {@link DEFAULT_LOCALE}. We do
 * NOT consult `navigator.language` here — on Android WebView that reflects the
 * app locale, which on no-GMS OEM builds is frequently English even when the
 * system is Chinese, so it is a less faithful guess than the plain default. The
 * boot IPC reconcile corrects any mismatch within a frame regardless.
 */
export function resolveBootLocale(): SupportedLocale {
  const injected = (globalThis as GpmLocaleGlobals).__GPM_LOCALE__;
  return injected ? normalizeSupported(injected) : DEFAULT_LOCALE;
}

/** The vue-i18n instance. `legacy: false` ⇒ Composition API (`useI18n()`).
 *  The `false` generic makes `i18n.global` a Composer (so `locale` is a ref);
 *  `string` for the Locales slot lets messages start partial (only `en` is
 *  inlined; `zh-CN` loads lazily) instead of requiring every locale upfront. */
export const i18n = createI18n<{ message: MessageSchema }, string, false>({
  legacy: false,
  locale: resolveBootLocale(),
  fallbackLocale: DEFAULT_LOCALE,
  messages: { en: { common: enCommon } },
});

/**
 * Lazily merge a JSON bundle (`<locale>/<namespace>.json`) into the global
 * messages, skipping the import when the namespace is already loaded for that
 * locale. A missing bundle (e.g. a page not yet translated for a locale) is
 * swallowed — `fallbackLocale` covers the missing keys.
 */
export async function loadBundle(
  locale: SupportedLocale,
  namespace: string,
): Promise<void> {
  const messages = i18n.global.getLocaleMessage(locale) as Record<
    string,
    unknown
  >;
  // Presence-based dedup: once a namespace has been loaded for this locale —
  // even as an empty bundle (e.g. a page not yet translated for it) — skip the
  // re-import. A non-empty check would re-import forever for intentionally-empty
  // bundles (the default-locale `common` ships as `{}` until keys are added).
  if (namespace in messages) return;
  try {
    const mod = await import(`@/locales/${locale}/${namespace}.json`);
    i18n.global.mergeLocaleMessage(locale, { [namespace]: mod.default });
  } catch {
    // Bundle not shipped for this locale/namespace — fallbackLocale covers it.
  }
}

/** The locale the app is currently rendering in. */
export function currentLocale(): SupportedLocale {
  return i18n.global.locale.value as SupportedLocale;
}

/**
 * Switch the active locale, loading its `common` bundle first so the first
 * render in the new locale has nav/buttons translated, and mirror it to
 * `<html lang>` for accessibility and `:lang()` CSS.
 */
export async function setLocale(locale: SupportedLocale): Promise<void> {
  await loadBundle(locale, "common");
  // Reload the page bundles the user is already viewing (the namespaces loaded
  // for the previous locale) so a locale switch translates the current page in
  // place, not just on the next navigation.
  const prev = i18n.global.locale.value;
  if (prev !== locale) {
    const loaded = i18n.global.getLocaleMessage(prev) as Record<
      string,
      unknown
    >;
    await Promise.all(Object.keys(loaded).map((ns) => loadBundle(locale, ns)));
  }
  i18n.global.locale.value = locale;
  document.documentElement.lang = locale;
}

/**
 * Ask the backend for the authoritative resolved locale and correct if it
 * differs from the boot assumption. A no-op on devices where the injected value
 * already matched (the common, track-system case); the one-frame correction
 * path for pinned-preference devices. Failures (e.g. no backend in pure-Vite
 * dev) are swallowed so the app keeps the boot locale.
 */
export async function reconcileLocaleFromBackend(): Promise<void> {
  try {
    const target = normalizeSupported(await resolvedLocaleApi());
    if (i18n.global.locale.value !== target) {
      await setLocale(target);
    }
  } catch {
    // No backend / IPC failed — keep the boot locale.
  }
}
