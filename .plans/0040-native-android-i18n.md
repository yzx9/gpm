# Internationalization for the Native Android Layer

**Priority:** P3
**Status:** Draft
**Phase:** Future

## What

Internationalize the native Android user-facing text that lives outside the
WebView — today the biometric prompt raised by the app-launch biometric gate and
the identity biometric unlock, whose subtitle and negative button are hardcoded
English in the native plugin layer. Drive it by the same resolved locale the
rest of the app uses, so the native prompt never disagrees with the WebView UI.
Deferred relative to the WebView layer (0039).

## Why

The WebView i18n (0039) does not reach native Android dialogs. The biometric
prompt is a platform dialog whose text is supplied by the native layer, not a
WebView string; a user who chose a language in-app that differs from the
device's system language would otherwise see the prompt in the wrong language —
the WebView in Chinese, the prompt in English. That split is most visible on the
lock screen, the surface the user stares at during cold start.

## Context

**Surface.** The prompt text is produced in the native plugin layer and cannot
be translated by 0039's message bundles. The remedy is to localize those strings
— Android string resources, or localized strings passed in when the prompt is
built — and select them by the in-app locale.

**Locale source — the shared backend resolver, not the system locale.** Android
resources follow the device's system locale by default, which is wrong whenever
the user overrides the language in-app. The prompt must use the *resolved in-app
locale* — the same value 0039's backend resolver computes for the WebView. The
mechanism used to select strings by that locale (a locale-overridden resource
context, or passing the localized strings in directly at prompt-build time) is
an implementation choice for when this RFC is picked up; the fixed decision is
the locale source, which removes the WebView/native split by construction.

**Threat model.** No change. The prompt text is non-sensitive UI copy.

## Alternatives considered

- **Let the prompt follow the device system locale.** Rejected: it mismatches
  the in-app locale whenever the user overrides the language, splitting the lock
  experience across two languages — the exact problem this RFC exists to close.

## Effort

~S (human) / ~S (CC): one localized string set per locale for the prompt, plus
routing the backend's resolved locale to the native prompt. No crypto, no
threat-model change. Deferred only because the WebView layer (0039) ships first.

## Depends on / Supersedes

Consumes the resolved-locale resolver established by 0039; independent of 0038's
plaintext-vs-sealed application-store decision (the preference's protection
level does not affect how the native layer reads the resolved value).
