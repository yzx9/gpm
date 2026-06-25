// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import type { AppLockState, AppLockError } from "./types";

/**
 * Thin wrappers over the app-launch biometric gate commands in
 * `src-tauri/src/applock.rs`. Like `biometric.ts`, the frontend never talks to
 * the keystore plugin directly — the master key flows Kotlin → Rust → Store and
 * never reaches the WebView. Availability/state probes swallow errors and return
 * a safe "off" shape on desktop / below API 30 / when the plugin is absent.
 */

/** Re-export so callers can type-narrow caught errors uniformly. */
export type { AppLockError };

/**
 * Whether the app-launch gate is usable (API 30+ with STRONG biometric). `false`
 * on desktop / Android <11. Surfaces as "off" on any error.
 */
export async function isAppLockAvailable(): Promise<boolean> {
  try {
    return await invoke<boolean>("is_app_lock_available");
  } catch {
    return false;
  }
}

/**
 * Current app-lock state. Defaults to disabled/unlocked on error (desktop,
 * pre-setup) so the UI never shows the gate overlay spuriously.
 */
export async function getAppLockState(): Promise<AppLockState> {
  try {
    return await invoke<AppLockState>("get_app_lock_state");
  } catch {
    return { enabled: false, locked: false };
  }
}

/**
 * Enable the gate: migrate the at-rest master key behind biometric. Shows a
 * BiometricPrompt; rejects with {@link AppLockError} on cancel/failure.
 */
export async function enableBiometricAppLock(): Promise<void> {
  await invoke("enable_biometric_app_lock");
}

/**
 * Disable the gate: migrate the master key back to the auth-free store (one last
 * BiometricPrompt). Rejects with {@link AppLockError} on cancel/failure.
 */
export async function disableBiometricAppLock(): Promise<void> {
  await invoke("disable_biometric_app_lock");
}

/**
 * Unlock the app via biometric (CryptoObject DECRYPT of the master key). Shows a
 * BiometricPrompt; resolves on success, rejects with {@link AppLockError} on
 * cancel/failure. Idempotent if already unlocked.
 */
export async function appUnlock(): Promise<void> {
  await invoke("app_unlock");
}

/**
 * Re-lock the app (wipe the master key + identity). Never rejects — the frontend
 * calls this on app resume to re-raise the gate.
 */
export async function appLock(): Promise<void> {
  try {
    await invoke("app_lock");
  } catch {
    // Best-effort: a failed re-lock still leaves the overlay state to the
    // backend event, which is the source of truth.
  }
}

/** Type-narrow a caught value into an {@link AppLockError}. */
export function asAppLockError(e: unknown): AppLockError {
  return e as AppLockError;
}
