// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import type { BiometricError } from "./types";

/**
 * Thin wrappers over the five biometric app commands in `src-tauri/src/lib.rs`.
 *
 * The frontend never talks to `plugin:biometric-keystore|*` directly — all secret-
 * returning operations stay backend-side so passphrases never reach the
 * WebView. `isBiometricAvailable` / `isBiometricUnlockEnabled` swallow errors
 * and return `false` on desktop / below API 30 / when the plugin is absent,
 * so callers can treat biometric as simply "off" there.
 */

/**
 * Whether biometric-gated storage is usable on this device (API 30+ with a
 * STRONG biometric enrolled). `false` on desktop and Android <11.
 */
export async function isBiometricAvailable(): Promise<boolean> {
  try {
    return await invoke<boolean>("is_biometric_available");
  } catch {
    return false;
  }
}

/**
 * Whether a passphrase is sealed in the Keystore — the single source of truth
 * for "biometric is enabled" (there is no flag file). `false` on desktop.
 */
export async function isBiometricUnlockEnabled(): Promise<boolean> {
  try {
    return await invoke<boolean>("is_biometric_unlock_enabled");
  } catch {
    return false;
  }
}

/**
 * Enable biometric unlock: validates `passphrase` (rejecting a wrong one),
 * then seals it behind a biometric prompt (CryptoObject ENCRYPT). Rejects with
 * a {@link BiometricError} on failure (e.g. `WRONG_PASSPHRASE`,
 * `BIOMETRIC_CANCELLED`).
 */
export async function enableBiometricUnlock(passphrase: string): Promise<void> {
  await invoke("enable_biometric_unlock", { passphrase });
}

/**
 * Unlock via biometrics: shows a biometric prompt, retrieves the sealed
 * passphrase, and runs it through the same unlock path as the password UI.
 * Resolves on success; rejects with a {@link BiometricError} on cancel or
 * failure.
 */
export async function biometricUnlock(): Promise<void> {
  await invoke("biometric_unlock");
}

/**
 * Disable biometric unlock (best-effort). Never rejects — disabling must
 * always succeed so the user can escape a stuck state.
 */
export async function disableBiometricUnlock(): Promise<void> {
  try {
    await invoke("disable_biometric_unlock");
  } catch {
    // Best-effort.
  }
}

/** Type-narrow a caught value into a {@link BiometricError}. */
export function asBiometricError(e: unknown): BiometricError {
  return e as BiometricError;
}
