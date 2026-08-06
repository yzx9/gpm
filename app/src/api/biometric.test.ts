// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  biometricUnlock,
  disableBiometricUnlock,
  enableBiometricUnlock,
  isBiometricAvailable,
  isBiometricUnlockEnabled,
  openSecuritySettings,
} from "./biometric";

vi.mock("@tauri-apps/api/core");

describe("biometric wrappers", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("isBiometricAvailable returns the quad-state from is_biometric_available", async () => {
    for (const state of [
      "available",
      "no_enrollment",
      "weak_enrolled",
      "unavailable",
    ] as const) {
      (invoke as ReturnType<typeof vi.fn>).mockResolvedValue(state);
      expect(await isBiometricAvailable()).toBe(state);
    }
    expect(invoke).toHaveBeenCalledWith("is_biometric_available");
  });

  it("isBiometricAvailable swallows errors and returns 'unavailable' (desktop / <API30)", async () => {
    (invoke as ReturnType<typeof vi.fn>).mockRejectedValue(
      new Error("plugin not found"),
    );
    expect(await isBiometricAvailable()).toBe("unavailable");
  });

  it("openSecuritySettings calls the open_security_settings command", async () => {
    (invoke as ReturnType<typeof vi.fn>).mockResolvedValue(true);
    expect(await openSecuritySettings()).toBe(true);
    expect(invoke).toHaveBeenCalledWith("open_security_settings");
  });

  it("isBiometricUnlockEnabled calls the is_biometric_unlock_enabled command", async () => {
    (invoke as ReturnType<typeof vi.fn>).mockResolvedValue(true);
    expect(await isBiometricUnlockEnabled()).toBe(true);
    expect(invoke).toHaveBeenCalledWith("is_biometric_unlock_enabled");
  });

  it("isBiometricUnlockEnabled swallows errors and returns false", async () => {
    (invoke as ReturnType<typeof vi.fn>).mockRejectedValue(
      new Error("plugin not found"),
    );
    expect(await isBiometricUnlockEnabled()).toBe(false);
  });

  it("enableBiometricUnlock passes the passphrase through", async () => {
    (invoke as ReturnType<typeof vi.fn>).mockResolvedValue(undefined);
    await enableBiometricUnlock("hunter2");
    expect(invoke).toHaveBeenCalledWith("enable_biometric_unlock", {
      passphrase: "hunter2",
    });
  });

  it("enableBiometricUnlock propagates rejection (e.g. wrong passphrase)", async () => {
    (invoke as ReturnType<typeof vi.fn>).mockRejectedValue({
      code: "WRONG_PASSPHRASE",
      message: "nope",
    });
    await expect(enableBiometricUnlock("x")).rejects.toEqual({
      code: "WRONG_PASSPHRASE",
      message: "nope",
    });
  });

  it("biometricUnlock calls the biometric_unlock command", async () => {
    (invoke as ReturnType<typeof vi.fn>).mockResolvedValue(undefined);
    await biometricUnlock();
    expect(invoke).toHaveBeenCalledWith("biometric_unlock", expect.anything());
  });

  it("biometricUnlock propagates rejection (e.g. cancel)", async () => {
    (invoke as ReturnType<typeof vi.fn>).mockRejectedValue({
      code: "KEYSTORE_CANCELLED",
      message: "cancel",
    });
    await expect(biometricUnlock()).rejects.toEqual({
      code: "KEYSTORE_CANCELLED",
      message: "cancel",
    });
  });

  it("disableBiometricUnlock calls the disable_biometric_unlock command", async () => {
    (invoke as ReturnType<typeof vi.fn>).mockResolvedValue(undefined);
    await disableBiometricUnlock();
    expect(invoke).toHaveBeenCalledWith("disable_biometric_unlock");
  });

  it("disableBiometricUnlock never rejects (best-effort)", async () => {
    (invoke as ReturnType<typeof vi.fn>).mockRejectedValue(new Error("boom"));
    await expect(disableBiometricUnlock()).resolves.toBeUndefined();
  });
});
