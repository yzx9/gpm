// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  areClipboardNotificationsEnabled,
  ensureClipboardNotifyPermission,
  requestClipboardNotificationsPermission,
} from "./clipboard";

vi.mock("@tauri-apps/api/core");

describe("clipboard notification wrappers", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("areClipboardNotificationsEnabled calls are_clipboard_notifications_enabled", async () => {
    (invoke as ReturnType<typeof vi.fn>).mockResolvedValue(true);
    expect(await areClipboardNotificationsEnabled()).toBe(true);
    expect(invoke).toHaveBeenCalledWith("are_clipboard_notifications_enabled");
  });

  it("requestClipboardNotificationsPermission calls request_clipboard_notifications_permission", async () => {
    (invoke as ReturnType<typeof vi.fn>).mockResolvedValue(true);
    expect(await requestClipboardNotificationsPermission()).toBe(true);
    expect(invoke).toHaveBeenCalledWith(
      "request_clipboard_notifications_permission",
    );
  });

  it("ensureClipboardNotifyPermission skips the request when already granted", async () => {
    // are_enabled -> true: the probe short-circuits before requesting.
    (invoke as ReturnType<typeof vi.fn>).mockResolvedValue(true);
    await ensureClipboardNotifyPermission();
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("are_clipboard_notifications_enabled");
    expect(invoke).not.toHaveBeenCalledWith(
      "request_clipboard_notifications_permission",
    );
  });

  it("ensureClipboardNotifyPermission requests the permission when not granted", async () => {
    (invoke as ReturnType<typeof vi.fn>).mockResolvedValue(false);
    await ensureClipboardNotifyPermission();
    expect(invoke).toHaveBeenCalledWith(
      "request_clipboard_notifications_permission",
    );
  });

  it("ensureClipboardNotifyPermission treats a denied dialog (granted=false) as success, not an error", async () => {
    // Denial is a normal outcome of the system dialog, not a throw — the copy
    // must still proceed. This is the contract the simplification rests on.
    (invoke as ReturnType<typeof vi.fn>).mockResolvedValue(false);
    await expect(ensureClipboardNotifyPermission()).resolves.toBeUndefined();
  });

  it("ensureClipboardNotifyPermission swallows a broken probe, still resolves, and does not request", async () => {
    (invoke as ReturnType<typeof vi.fn>).mockRejectedValue(
      new Error("plugin not found"),
    );
    await expect(ensureClipboardNotifyPermission()).resolves.toBeUndefined();
    expect(invoke).not.toHaveBeenCalledWith(
      "request_clipboard_notifications_permission",
    );
  });

  it("ensureClipboardNotifyPermission swallows a request-side rejection and still resolves", async () => {
    (invoke as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce(false)
      .mockRejectedValueOnce(new Error("request failed"));
    await expect(ensureClipboardNotifyPermission()).resolves.toBeUndefined();
  });
});
