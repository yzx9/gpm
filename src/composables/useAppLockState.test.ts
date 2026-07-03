// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createAppLockStore, type AppLockStore } from "./useAppLockState";

/** Force jsdom's visibilityState and fire the event the composable listens to. */
function setVisibility(state: "visible" | "hidden") {
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    value: state,
  });
  document.dispatchEvent(new Event("visibilitychange"));
}

describe("useAppLockState", () => {
  let s: AppLockStore;

  beforeEach(() => {
    vi.clearAllMocks();
    // Fresh per test — replaces the old module-singleton __resetAppLockStateForTests.
    s = createAppLockStore();
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      value: "visible",
    });
  });

  afterEach(() => {
    // Drop this instance's `visibilitychange` listener so it doesn't leak onto
    // the next test's instance (each test creates a fresh store).
    s.dispose();
  });

  it("is disabled, unlocked, and not ready until init() resolves", () => {
    expect(s.appLockEnabled.value).toBe(false);
    expect(s.appLocked.value).toBe(false);
    expect(s.appReady.value).toBe(false);
  });

  it("init() reflects an enabled+locked gate and flips ready", async () => {
    vi.mocked(invoke).mockResolvedValue({ enabled: true, locked: true });

    await s.init();

    expect(invoke).toHaveBeenCalledWith("get_app_lock_state");
    expect(s.appLockEnabled.value).toBe(true);
    expect(s.appLocked.value).toBe(true);
    expect(s.appReady.value).toBe(true);
  });

  it("init() defaults to disabled when get_app_lock_state rejects", async () => {
    vi.mocked(invoke).mockRejectedValue(new Error("boom"));

    await s.init();

    expect(s.appLockEnabled.value).toBe(false);
    expect(s.appLocked.value).toBe(false);
    expect(s.appReady.value).toBe(true);
  });

  it("init() registers a single app-lock-state listener and is idempotent", async () => {
    vi.mocked(invoke).mockResolvedValue({ enabled: false, locked: false });

    await s.init();
    await s.init();

    expect(listen).toHaveBeenCalledTimes(1);
    expect(listen).toHaveBeenCalledWith("app-lock-state", expect.any(Function));
  });

  it("the app-lock-state handler mirrors the backend payload", async () => {
    vi.mocked(invoke).mockResolvedValue({ enabled: true, locked: true });
    await s.init();

    const handler = vi.mocked(listen).mock.calls[0][1] as (e: {
      payload: { enabled: boolean; locked: boolean };
    }) => void;

    handler({ payload: { enabled: true, locked: false } });
    expect(s.appLocked.value).toBe(false);

    handler({ payload: { enabled: true, locked: true } });
    expect(s.appLocked.value).toBe(true);
  });

  it("resume (visibilitychange→visible) re-locks when enabled+unlocked", async () => {
    vi.mocked(invoke).mockImplementation((cmd) => {
      if (cmd === "get_app_lock_state")
        return Promise.resolve({ enabled: true, locked: false });
      return Promise.resolve();
    });
    await s.init();
    expect(s.appLockEnabled.value).toBe(true);
    expect(s.appLocked.value).toBe(false);

    vi.mocked(invoke).mockClear();
    setVisibility("visible");

    expect(invoke).toHaveBeenCalledWith("app_lock");
  });

  it("resume does NOT re-lock when the gate is disabled", async () => {
    vi.mocked(invoke).mockResolvedValue({ enabled: false, locked: false });
    await s.init();

    vi.mocked(invoke).mockClear();
    setVisibility("visible");

    expect(invoke).not.toHaveBeenCalledWith("app_lock");
  });

  it("resume does NOT re-lock when already locked", async () => {
    vi.mocked(invoke).mockResolvedValue({ enabled: true, locked: true });
    await s.init();

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
    await s.init();

    s.setUnlockInFlight(true);
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
    await s.init();

    vi.mocked(invoke).mockClear();
    setVisibility("hidden");

    expect(invoke).not.toHaveBeenCalledWith("app_lock");
  });

  it("resume is debounced right after an unlock (loop guard for prompt dismiss)", async () => {
    // Cold-start locked, then the backend reports an unlock (locked→false).
    vi.mocked(invoke).mockResolvedValue({ enabled: true, locked: true });
    await s.init();
    const handler = vi.mocked(listen).mock.calls[0][1] as (e: {
      payload: { enabled: boolean; locked: boolean };
    }) => void;
    handler({ payload: { enabled: true, locked: false } }); // unlock transition
    expect(s.appLocked.value).toBe(false);

    // A resume within the debounce window must NOT re-lock (the prompt's own
    // dismiss could otherwise re-trigger the gate in a loop).
    vi.mocked(invoke).mockClear();
    setVisibility("visible");
    expect(invoke).not.toHaveBeenCalledWith("app_lock");
  });
});
