// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createAppLockStore, type AppLockStore } from "./useAppLockState";

/** Resolve the `subscribeAppResume` handler captured on the mocked `listen` (the
 *  authoritative `app-resumed` signal, R029) and fire it, simulating an Android
 *  `Activity.onResume`. */
function fireResume() {
  const call = vi.mocked(listen).mock.calls.find((c) => c[0] === "app-resumed");
  // Fail loudly if the resume listener never registered — without this the
  // negative tests below pass vacuously (no handler to fire).
  expect(call).toBeDefined();
  (call?.[1] as () => void)?.();
}

describe("useAppLockState", () => {
  let s: AppLockStore;

  beforeEach(() => {
    vi.clearAllMocks();
    // Fresh per test — replaces the old module-singleton __resetAppLockStateForTests.
    s = createAppLockStore();
  });

  afterEach(() => {
    // Drop this instance's listeners so they don't leak onto the next test's
    // instance (each test creates a fresh store).
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

  it("init() registers the app-lock-state + app-resume listeners once each", async () => {
    vi.mocked(invoke).mockResolvedValue({ enabled: false, locked: false });

    await s.init();
    await s.init();

    expect(listen).toHaveBeenCalledTimes(2);
    expect(listen).toHaveBeenCalledWith("app-lock-state", expect.any(Function));
    expect(listen).toHaveBeenCalledWith("app-resumed", expect.any(Function));
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

  it("records the lock reason and gates the auto-prompt (idle suppresses)", async () => {
    vi.mocked(invoke).mockResolvedValue({ enabled: true, locked: true });
    await s.init();
    const handler = vi.mocked(listen).mock.calls[0][1] as (e: {
      payload: { enabled: boolean; locked: boolean; reason?: string | null };
    }) => void;

    // Cold start (reason null) → auto-prompt (today's behavior).
    handler({ payload: { enabled: true, locked: true, reason: null } });
    expect(s.shouldAutoPrompt.value).toBe(true);

    // A resume re-lock (reason "return") → auto-prompt.
    handler({ payload: { enabled: true, locked: true, reason: "return" } });
    expect(s.shouldAutoPrompt.value).toBe(true);

    // An idle re-lock (reason "idle") → suppress the auto-prompt (R057: the user
    // is present but idle, so the mask shows and they tap).
    handler({ payload: { enabled: true, locked: true, reason: "idle" } });
    expect(s.shouldAutoPrompt.value).toBe(false);
  });

  it("resume (app-resumed) re-locks when enabled+unlocked", async () => {
    vi.mocked(invoke).mockImplementation((cmd) => {
      if (cmd === "get_app_lock_state")
        return Promise.resolve({ enabled: true, locked: false });
      return Promise.resolve();
    });
    await s.init();
    expect(s.appLockEnabled.value).toBe(true);
    expect(s.appLocked.value).toBe(false);

    vi.mocked(invoke).mockClear();
    fireResume();

    expect(invoke).toHaveBeenCalledWith("app_lock");
  });

  it("resume does NOT re-lock when the gate is disabled", async () => {
    vi.mocked(invoke).mockResolvedValue({ enabled: false, locked: false });
    await s.init();

    vi.mocked(invoke).mockClear();
    fireResume();

    expect(invoke).not.toHaveBeenCalledWith("app_lock");
  });

  it("resume does NOT re-lock when already locked", async () => {
    vi.mocked(invoke).mockResolvedValue({ enabled: true, locked: true });
    await s.init();

    vi.mocked(invoke).mockClear();
    fireResume();

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
    fireResume();

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
    fireResume();
    expect(invoke).not.toHaveBeenCalledWith("app_lock");
  });
});
