# Internationalization for the WebView

**Priority:** P2
**Status:** Draft
**Phase:** Now

## What

Add internationalization to the WebView frontend — English as the default
locale, Chinese as the first additional one — using Vue I18n in Composition API
mode, with page-level lazy loading of message bundles, framework-backed
pluralization, and system-language tracking with manual override. The resolved
locale is injected into the WebView by the backend before the page's scripts
run, so the first frame is already in the right language; the frontend reads it
with a graceful fallback to its own system-language detection when no value was
injected. The locale preference is an application-scoped value (see 0038) and
lives in the plaintext application store. Native Android surfaces (the
biometric prompt) are out of scope here and are covered by 0040.

## Why

The frontend ships English-only with a large body of hardcoded user-facing
strings; for Chinese-speaking users it is effectively unusable, and the goal is
a second locale without architectural change. The interesting decisions are the
framework, how message bundles load, and how the chosen locale reaches the
frontend early enough that no wrong-language frame paints — without introducing
a second source of truth or a boot IPC fetch.

## Context

**Framework — Vue I18n, Composition API mode.** It is Vue's official library;
its composable API mirrors the codebase's existing composable + provide/inject
architecture; pluralization is backed by the platform's plural rules (English
one/other, Chinese single-form — no custom rules needed); message keys are
type-safe; and the official Vite plugin compiles bundles at build time, keeping
the runtime compiler out of the shipped bundle.

**Locale injection — backend-resolved, pre-paint.** The locale preference lives
in the plaintext application store, which the backend already reads at startup.
The backend resolves the final locale (an explicit choice, or the system
language when the preference is "track system") and injects it into the WebView
through Tauri's initialization-script mechanism, which runs before the page's
own scripts. The frontend therefore reads the locale synchronously at boot,
before mount, and the first frame renders in the correct language — no
wrong-language flash, no boot IPC fetch, no second source of truth.

**Graceful fallback.** The injected value is the fast path, not a dependency:
when no value is injected (development, a platform path that skips injection, or
a first run before any preference exists), the frontend falls back to detecting
the system language from the WebView's own locale. The app always renders in
some sensible locale; injection only removes the flash and makes the backend the
single resolver. This makes the mechanism robust rather than load-bearing — a
missing injection degrades to the same behavior an uninjected app would have,
never to a broken or blank first frame.

**Locale lifecycle.** The default is to track the system language, with manual
override to English or Chinese. The preference is application-scoped and
survives repository resets; until 0038 lands it stays in the existing plaintext
application store, and migrates to the sealed application store when 0038 lands
— injection is unaffected, since the backend still reads and resolves the
preference before the WebView loads either way.

**Lazy loading — page-level.** Each route's message bundle loads alongside the
route component, and a shared "common" bundle (navigation, generic buttons,
toasts) for the default locale loads at first paint. Page-level rather than
one-bundle-per-locale keeps the initial payload small — several pages are large
— and aligns message loading with the existing route structure.

**Pluralization** is left to the framework's plural rules; English and Chinese
are the two simplest cases, so no custom pluralization is needed now.

**Threat model.** No change. Locale is a non-sensitive UI preference.

## Alternatives considered

- **One message bundle per locale (not page-level).** Rejected: bundling every
  page's strings into a single per-locale file would load strings for pages (and
  a locale) the user may never visit, which several large pages make costly.
- **i18next with its Vue wrapper.** Rejected: its framework-agnostic ecosystem
  is irrelevant here — there is no third-party component library whose own i18n
  must stay in sync — so a non-official wrapper adds friction with no offsetting
  benefit.
- **FormatJS / ICU MessageFormat.** Rejected: heavier, with ICU syntax overhead
  and a smaller Vue community; the pluralization needs do not require ICU
  select/gender.
- **A boot IPC fetch, then reconcile.** Rejected as the primary path: it paints
  a default-language first frame and reconciles afterward, which — unlike the
  screen-capture toggle's invisible, safe-direction reconcile — is a _visible_
  wrong-language flash, because rendered text is visible and has no safe
  default. Injection removes that flash without a second store.
- **A WebView-side locale cache (localStorage).** Rejected: injection achieves
  the same zero-flash result with a single source of truth (the backend),
  avoiding a second store that can drift and must self-heal.

## Effort

~L (human) / ~M (CC). Framework wiring, page-level lazy loading, pluralization,
and the backend locale resolution + injection are moderate; the dominant cost is
mechanically extracting a large body of hardcoded strings across pages and
composables, done incrementally with the default locale as fallback so
untranslated keys degrade gracefully rather than block.

## Depends on / Supersedes

Locale persistence follows 0038 (application store; plaintext now, sealed later
— injection is unaffected either way). The native Android biometric-prompt text
is a separate i18n surface, deferred to 0040.
