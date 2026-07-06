// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/**
 * Thin wrappers over the app-launch biometric gate commands in
 * `src-tauri/src/applock.rs`. Like `biometric.ts`, the frontend never talks to
 * the keystore plugin directly — the master key flows Kotlin → Rust → Store and
 * never reaches the WebView. Availability/state probes swallow errors and return
 * a safe "off" shape on desktop / below API 30 / when the plugin is absent.
 */

/** App-launch biometric gate state from get_app_lock_state / the
 * `app-lock-state` event. `enabled` = the gate is on (master key is
 * biometric-gated); `locked` = the master key is not in memory (cold start or
 * after a background re-lock), so the app-lock overlay should be shown. */
export interface AppLockState {
  enabled: boolean;
  locked: boolean;
}

/** Error from the app-launch gate commands (`APP_LOCK_FAILED`, `BIOMETRIC_*`,
 * `SECURE_KEYSTORE_*`, or a `rustpass` code). Same shape as BiometricError. */
export interface AppLockError {
  code: string;
  message: string;
}

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
 * Enable the gate: migrate the seal master key behind biometric. Shows a
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

/**
 * Enable the identity-auto-unlock opt-in (req3): validate `passphrase` and seal
 * it under the master key so a later app-unlock also unlocks the identity with no
 * second prompt. Requires the gate on + an encrypted identity. Rejects with
 * {@link AppLockError} on a wrong passphrase or cancel.
 */
export async function enableIdentityAutoUnlock(
  passphrase: string,
): Promise<void> {
  await invoke("enable_identity_auto_unlock", { passphrase });
}

/**
 * Disable the identity-auto-unlock opt-in: clear the sealed passphrase slot +
 * the flag. Best-effort.
 */
export async function disableIdentityAutoUnlock(): Promise<void> {
  try {
    await invoke("disable_identity_auto_unlock");
  } catch {
    // Best-effort: the toggle reflects the persisted flag either way.
  }
}

/** Type-narrow a caught value into an {@link AppLockError}. */
export function asAppLockError(e: unknown): AppLockError {
  return e as AppLockError;
}

/** Subscribe to backend app-lock gate transitions (the same
 *  {@link AppLockState} shape `getAppLockState` returns). Returns an unlisten. */
export async function subscribeAppLockState(
  cb: (e: AppLockState) => void,
): Promise<UnlistenFn> {
  return listen<AppLockState>("app-lock-state", (e) => cb(e.payload));
}
