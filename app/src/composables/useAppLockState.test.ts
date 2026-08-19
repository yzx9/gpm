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

    // The appLocked guard is restored: a warm resume into an already-locked app
    // (e.g. the idle timer fired while away) does NOT ping app_lock — the backend's
    // apply_resume_relock is a no-op when locked anyway, and skipping avoids a
    // spurious cold-start ping that could race a just-finished unlock (R058 review).
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

  // ── onAppLock: the gate lock-edge signal for the eager-secret wipers ──────
  // (issue #20 — a gate re-lock raises the mask but does not unmount the page
  // underneath; the wipers subscribe to this edge to clear in-DOM secrets.)

  /** Resolve the `app-lock-state` handler captured on the mocked `listen`. */
  function gateHandler() {
    const call = vi
      .mocked(listen)
      .mock.calls.find((c) => c[0] === "app-lock-state");
    expect(call).toBeDefined();
    return call?.[1] as (e: {
      payload: { enabled: boolean; locked: boolean; reason?: string | null };
    }) => void;
  }

  it("onAppLock fires on the unlock→locked edge, not on unlock or locked→locked", async () => {
    vi.mocked(invoke).mockResolvedValue({ enabled: true, locked: false });
    await s.init();
    const cb = vi.fn();
    const off = s.onAppLock(cb);

    gateHandler()({ payload: { enabled: true, locked: true, reason: "idle" } });
    expect(cb).toHaveBeenCalledTimes(1);
    gateHandler()({
      payload: { enabled: true, locked: true, reason: "return" },
    });
    expect(cb).toHaveBeenCalledTimes(1); // locked→locked: no re-fire
    gateHandler()({ payload: { enabled: true, locked: false } });
    expect(cb).toHaveBeenCalledTimes(1); // unlock edge: no fire
    off();
  });

  it("onAppLock fires once on the cold-start reconcile into a locked gate", async () => {
    // The store starts unlocked by default; init() reconciling to locked:true
    // is an unlock→locked edge and fires — harmless (wipers are idempotent,
    // fresh pages hold nothing), pinned here so the behavior is deliberate.
    const cb = vi.fn();
    s.onAppLock(cb);
    vi.mocked(invoke).mockResolvedValue({ enabled: true, locked: true });
    await s.init();
    expect(cb).toHaveBeenCalledTimes(1);
  });

  it("one clearer throwing does not block the others (per-cb try/catch)", async () => {
    vi.mocked(invoke).mockResolvedValue({ enabled: true, locked: false });
    await s.init();
    const boom = vi.fn(() => {
      throw new Error("boom");
    });
    const ok = vi.fn();
    s.onAppLock(boom);
    s.onAppLock(ok);
    gateHandler()({ payload: { enabled: true, locked: true, reason: "idle" } });
    expect(boom).toHaveBeenCalledTimes(1);
    expect(ok).toHaveBeenCalledTimes(1);
  });

  it("onAppLock unsubscribe stops delivery", async () => {
    vi.mocked(invoke).mockResolvedValue({ enabled: true, locked: false });
    await s.init();
    const cb = vi.fn();
    const off = s.onAppLock(cb);
    off();
    gateHandler()({ payload: { enabled: true, locked: true, reason: "idle" } });
    expect(cb).not.toHaveBeenCalled();
  });

  it("dispose() drops the onAppLock listeners (no cross-instance leak)", async () => {
    vi.mocked(invoke).mockResolvedValue({ enabled: true, locked: false });
    await s.init();
    const cb = vi.fn();
    s.onAppLock(cb);
    s.dispose();
    gateHandler()({ payload: { enabled: true, locked: true, reason: "idle" } });
    expect(cb).not.toHaveBeenCalled();
  });

  it("setAppLocked drives the same path as the backend event (test driver)", async () => {
    vi.mocked(invoke).mockResolvedValue({ enabled: true, locked: false });
    await s.init();
    const cb = vi.fn();
    s.onAppLock(cb);

    s.setAppLocked(true, "idle");
    expect(cb).toHaveBeenCalledTimes(1);
    expect(s.appLocked.value).toBe(true);
    expect(s.shouldAutoPrompt.value).toBe(false); // reason recorded too

    s.setAppLocked(false);
    expect(cb).toHaveBeenCalledTimes(1); // unlock edge: no fire
  });
});
