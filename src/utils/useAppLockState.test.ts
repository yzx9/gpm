// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect, vi, beforeEach } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  __resetAppLockStateForTests,
  useAppLockState,
} from "./useAppLockState";

/** Force jsdom's visibilityState and fire the event the composable listens to. */
function setVisibility(state: "visible" | "hidden") {
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    value: state,
  });
  document.dispatchEvent(new Event("visibilitychange"));
}

describe("useAppLockState", () => {
  const { appLockEnabled, appLocked, appReady, init, setUnlockInFlight } =
    useAppLockState();

  beforeEach(() => {
    vi.clearAllMocks();
    __resetAppLockStateForTests();
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      value: "visible",
    });
  });

  it("is disabled, unlocked, and not ready until init() resolves", () => {
    expect(appLockEnabled.value).toBe(false);
    expect(appLocked.value).toBe(false);
    expect(appReady.value).toBe(false);
  });

  it("init() reflects an enabled+locked gate and flips ready", async () => {
    vi.mocked(invoke).mockResolvedValue({ enabled: true, locked: true });

    await init();

    expect(invoke).toHaveBeenCalledWith("get_app_lock_state");
    expect(appLockEnabled.value).toBe(true);
    expect(appLocked.value).toBe(true);
    expect(appReady.value).toBe(true);
  });

  it("init() defaults to disabled when get_app_lock_state rejects", async () => {
    vi.mocked(invoke).mockRejectedValue(new Error("boom"));

    await init();

    expect(appLockEnabled.value).toBe(false);
    expect(appLocked.value).toBe(false);
    expect(appReady.value).toBe(true);
  });

  it("init() registers a single app-lock-state listener and is idempotent", async () => {
    vi.mocked(invoke).mockResolvedValue({ enabled: false, locked: false });

    await init();
    await init();

    expect(listen).toHaveBeenCalledTimes(1);
    expect(listen).toHaveBeenCalledWith("app-lock-state", expect.any(Function));
  });

  it("the app-lock-state handler mirrors the backend payload", async () => {
    vi.mocked(invoke).mockResolvedValue({ enabled: true, locked: true });
    await init();

    const handler = vi.mocked(listen).mock.calls[0][1] as (e: {
      payload: { enabled: boolean; locked: boolean };
    }) => void;

    handler({ payload: { enabled: true, locked: false } });
    expect(appLocked.value).toBe(false);

    handler({ payload: { enabled: true, locked: true } });
    expect(appLocked.value).toBe(true);
  });

  it("resume (visibilitychange→visible) re-locks when enabled+unlocked", async () => {
    vi.mocked(invoke).mockImplementation((cmd) => {
      if (cmd === "get_app_lock_state")
        return Promise.resolve({ enabled: true, locked: false });
      return Promise.resolve();
    });
    await init();
    expect(appLockEnabled.value).toBe(true);
    expect(appLocked.value).toBe(false);

    vi.mocked(invoke).mockClear();
    setVisibility("visible");

    expect(invoke).toHaveBeenCalledWith("app_lock");
  });

  it("resume does NOT re-lock when the gate is disabled", async () => {
    vi.mocked(invoke).mockResolvedValue({ enabled: false, locked: false });
    await init();

    vi.mocked(invoke).mockClear();
    setVisibility("visible");

    expect(invoke).not.toHaveBeenCalledWith("app_lock");
  });

  it("resume does NOT re-lock when already locked", async () => {
    vi.mocked(invoke).mockResolvedValue({ enabled: true, locked: true });
    await init();

    vi.mocked(invoke).mockClear();
    setVisibility("visible");

    expect(invoke).not.toHaveBeenCalledWith("app_lock");
  });

  it("resume does NOT re-lock while a biometric unlock is in flight (loop guard)", async () => {
    vi.mocked(invoke).mockImplementation((cmd) => {
      if (cmd === "get_app_lock_state")
        return Promise.resolve({ enabled: true, locked: false });
      return Promise.resolve();
    });
    await init();

    setUnlockInFlight(true);
    vi.mocked(invoke).mockClear();
    setVisibility("visible");

    expect(invoke).not.toHaveBeenCalledWith("app_lock");
  });

  it("resume ignores the hidden half of visibilitychange", async () => {
    vi.mocked(invoke).mockImplementation((cmd) => {
      if (cmd === "get_app_lock_state")
        return Promise.resolve({ enabled: true, locked: false });
      return Promise.resolve();
    });
    await init();

    vi.mocked(invoke).mockClear();
    setVisibility("hidden");

    expect(invoke).not.toHaveBeenCalledWith("app_lock");
  });

  it("resume is debounced right after an unlock (loop guard for prompt dismiss)", async () => {
    // Cold-start locked, then the backend reports an unlock (locked→false).
    vi.mocked(invoke).mockResolvedValue({ enabled: true, locked: true });
    await init();
    const handler = vi.mocked(listen).mock.calls[0][1] as (e: {
      payload: { enabled: boolean; locked: boolean };
    }) => void;
    handler({ payload: { enabled: true, locked: false } }); // unlock transition
    expect(appLocked.value).toBe(false);

    // A resume within the debounce window must NOT re-lock (the prompt's own
    // dismiss could otherwise re-trigger the gate in a loop).
    vi.mocked(invoke).mockClear();
    setVisibility("visible");
    expect(invoke).not.toHaveBeenCalledWith("app_lock");
  });
});
