// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { i18n } from "./index";

/**
 * Native-Android-prompt text, owned by the frontend.
 *
 * The native Kotlin layer (BiometricPrompt, the clipboard notification, the
 * NotificationChannel) does no localization. The frontend reads the single
 * source — `src/locales/<locale>/native.json` — and passes already-localized
 * strings through the Tauri commands. These builders centralize the message
 * keys so each surface's text is declared in exactly one place (the bundle) and
 * each key in exactly one place (here). Call sites call the builder that matches
 * the operation they're raising, then forward the result to the API wrapper.
 */

/**
 * `t()` that yields `undefined` on a miss (key absent or bundle not loaded), so
 * the native layer's generic fallback fires. vue-i18n's `t()` would otherwise
 * return the raw key path (e.g. "native.biometric.identity.unlockTitle"), which
 * is non-blank and so bypasses the Kotlin blank-check fallback — a failed bundle
 * load would render a key path in the biometric prompt instead of the generic
 * safety title.
 */
function tx(key: string): string | undefined {
  return i18n.global.te(key) ? i18n.global.t(key) : undefined;
}

/**
 * Localized BiometricPrompt title/subtitle/negative-button. A field is
 * `undefined` when its bundle key is missing (bundle not loaded) — the native
 * layer then falls back to a generic safety string, never a key path.
 */
export interface BiometricPromptText {
  title: string | undefined;
  subtitle: string | undefined;
  negative: string | undefined;
}

/**
 * Localized clipboard-clear notification text. `bodyTemplate` carries a `{secs}`
 * hole; Rust substitutes the auto-clear window at post time (it owns `secs`),
 * so this is the RAW message (`tm`, not `t`) — interpolating here would consume
 * the token before Rust sees it. Fields are `undefined` when the bundle is
 * missing (native layer falls back).
 */
export interface ClipboardNotifyText {
  title: string | undefined;
  bodyTemplate: string | undefined;
  channelName: string | undefined;
  channelDescription: string | undefined;
}

/** Identity biometric enrollment prompt (`enable_biometric_unlock`). */
export function identityEnrollPrompt(): BiometricPromptText {
  return {
    title: tx("native.biometric.identity.enrollTitle"),
    subtitle: tx("native.biometric.subtitle"),
    negative: tx("native.biometric.identity.negative"),
  };
}

/** Identity biometric unlock prompt (`biometric_unlock`). */
export function identityUnlockPrompt(): BiometricPromptText {
  return {
    title: tx("native.biometric.identity.unlockTitle"),
    subtitle: tx("native.biometric.subtitle"),
    negative: tx("native.biometric.identity.negative"),
  };
}

/** App-lock enrollment prompt (`enable_biometric_app_lock`). */
export function appLockEnrollPrompt(): BiometricPromptText {
  return {
    title: tx("native.biometric.appLock.enrollTitle"),
    subtitle: tx("native.biometric.subtitle"),
    negative: tx("native.biometric.appLock.negative"),
  };
}

/** App-lock unlock / disable prompt (`app_unlock`, `disable_biometric_app_lock`). */
export function appLockUnlockPrompt(): BiometricPromptText {
  return {
    title: tx("native.biometric.appLock.unlockTitle"),
    subtitle: tx("native.biometric.subtitle"),
    negative: tx("native.biometric.appLock.negative"),
  };
}

/** Clipboard-clear notification text (title, body template, channel name/desc). */
export function clipboardNotifyText(): ClipboardNotifyText {
  const bodyKey = "native.clipboard.autoClearBody";
  // `tm` returns the raw message (no interpolation), so the `{secs}` hole
  // survives for Rust to substitute at post time. Gated by `te` so a missing
  // bundle yields `undefined` (→ native fallback) rather than a key-path string.
  const bodyTemplate = i18n.global.te(bodyKey)
    ? (i18n.global.tm(bodyKey) as string)
    : undefined;
  return {
    title: tx("native.clipboard.title"),
    bodyTemplate,
    channelName: tx("native.clipboard.channelName"),
    channelDescription: tx("native.clipboard.channelDescription"),
  };
}
