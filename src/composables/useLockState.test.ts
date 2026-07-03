// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { withSetup } from "@/test/withSetup";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  AUTH_CANCELLED,
  createLockState,
  isAuthCancelled,
  LOCK_KEY,
  useLockState,
  type LockState,
} from "./useLockState";

describe("useLockState", () => {
  let s: LockState;

  beforeEach(() => {
    vi.clearAllMocks();
    // Fresh, fail-closed, not ready — per-test isolation replaces the old
    // module-singleton __resetLockStateForTests.
    s = createLockState();
  });

  it("is fail-closed (locked) and not ready until init() resolves", () => {
    expect(s.locked.value).toBe(true);
    expect(s.ready.value).toBe(false);
  });

  it("init() reflects encrypted+locked auth state and flips ready", async () => {
    vi.mocked(invoke).mockResolvedValue({
      configured: true,
      encrypted: true,
      unlocked: false,
      identity_type: "x25519",
    });

    await s.init();

    expect(invoke).toHaveBeenCalledWith("get_auth_state");
    expect(s.locked.value).toBe(true);
    expect(s.ready.value).toBe(true);
  });

  it("init() unlocks for an encrypted-but-already-unlocked identity", async () => {
    vi.mocked(invoke).mockResolvedValue({
      configured: true,
      encrypted: true,
      unlocked: true,
      identity_type: "x25519",
    });

    await s.init();

    expect(s.locked.value).toBe(false);
  });

  it("init() unlocks for an unencrypted identity (nothing to lock)", async () => {
    vi.mocked(invoke).mockResolvedValue({
      configured: true,
      encrypted: false,
      unlocked: false,
      identity_type: "x25519",
    });

    await s.init();

    expect(s.locked.value).toBe(false);
  });

  it("init() stays fail-closed when get_auth_state rejects", async () => {
    vi.mocked(invoke).mockRejectedValue(new Error("boom"));

    await s.init();

    expect(s.locked.value).toBe(true);
  });

  it("init() registers a single identity-lock-state listener and is idempotent", async () => {
    vi.mocked(invoke).mockResolvedValue({
      configured: true,
      encrypted: true,
      unlocked: true,
      identity_type: "x25519",
    });

    await s.init();
    await s.init();

    expect(listen).toHaveBeenCalledTimes(1);
    expect(listen).toHaveBeenCalledWith(
      "identity-lock-state",
      expect.any(Function),
    );
  });

  it("the identity-lock-state handler mirrors the backend payload", async () => {
    vi.mocked(invoke).mockResolvedValue({
      configured: true,
      encrypted: true,
      unlocked: true,
      identity_type: "x25519",
    });
    await s.init();
    expect(s.locked.value).toBe(false);

    const handler = vi.mocked(listen).mock.calls[0][1] as (e: {
      payload: { locked: boolean };
    }) => void;

    // Backend says locked → mirror it (and fire clear-on-lock).
    handler({ payload: { locked: true } });
    expect(s.locked.value).toBe(true);

    // Backend says unlocked → mirror it.
    handler({ payload: { locked: false } });
    expect(s.locked.value).toBe(false);
  });

  it("setLocked(true) fires onLock callbacks AFTER flipping the ref", () => {
    const seen: boolean[] = [];
    s.onLock(() => seen.push(s.locked.value));

    s.setLocked(false);
    expect(seen).toEqual([]); // unlocking fires nothing

    s.setLocked(true);
    expect(seen).toEqual([true]); // fired, and saw the already-flipped ref
  });

  it("setLocked with the same value is a no-op (re-lock while open)", () => {
    const cb = vi.fn();
    s.onLock(cb);

    s.setLocked(true); // default was already true → no transition
    expect(cb).not.toHaveBeenCalled();
  });

  it("a throwing clearer does not block subsequent clearers", () => {
    const second = vi.fn();
    s.onLock(() => {
      throw new Error("boom");
    });
    s.onLock(second);

    s.setLocked(false);
    s.setLocked(true);

    expect(second).toHaveBeenCalledTimes(1);
  });

  it("onLock returns an unsubscribe that removes the callback", () => {
    const cb = vi.fn();
    const off = s.onLock(cb);

    s.setLocked(false);
    off();
    s.setLocked(true);

    expect(cb).not.toHaveBeenCalled();
  });

  // ── no-cache (Immediate) split: identityCached vs overlay ───────────────

  it("a soft wipe empties the cache without raising the overlay or firing onLock", async () => {
    vi.mocked(invoke).mockResolvedValue({
      configured: true,
      encrypted: true,
      unlocked: true,
      identity_type: "x25519",
    });
    await s.init();
    expect(s.identityCached.value).toBe(true);
    expect(s.locked.value).toBe(false);

    const clearer = vi.fn();
    s.onLock(clearer);

    const handler = vi.mocked(listen).mock.calls[0][1] as (e: {
      payload: { locked: boolean; soft?: boolean };
    }) => void;
    // Soft wipe: the identity leaves the cache, but the overlay stays down and
    // onLock must NOT fire (a revealed secret / draft survives it).
    handler({ payload: { locked: true, soft: true } });
    expect(s.identityCached.value).toBe(false);
    expect(s.locked.value).toBe(false);
    expect(s.overlayUp.value).toBe(false);
    expect(clearer).not.toHaveBeenCalled();
  });

  it("runWithAuth runs the op immediately when the identity is cached", async () => {
    s = createLockState({ unlocked: true }); // replaces __unlockForTests
    const op = vi.fn().mockResolvedValue("done");
    await expect(s.runWithAuth(op)).resolves.toBe("done");
    expect(op).toHaveBeenCalledTimes(1);
    expect(s.overlayUp.value).toBe(false);
  });

  it("runWithAuth prompts, then resumes after an unlock when not cached", async () => {
    vi.mocked(invoke).mockResolvedValue({
      configured: true,
      encrypted: true,
      unlocked: false,
      identity_type: "x25519",
    });
    await s.init();
    expect(s.identityCached.value).toBe(false);

    const op = vi.fn().mockResolvedValue("done");
    const p = s.runWithAuth(op);
    // Blocked: the per-op overlay is raised and the op has not run.
    expect(s.overlayUp.value).toBe(true);
    expect(op).not.toHaveBeenCalled();

    const handler = vi.mocked(listen).mock.calls[0][1] as (e: {
      payload: { locked: boolean; soft?: boolean };
    }) => void;
    handler({ payload: { locked: false } }); // backend reports unlock
    await expect(p).resolves.toBe("done");
    expect(op).toHaveBeenCalledTimes(1);
    expect(s.identityCached.value).toBe(true);
    expect(s.overlayUp.value).toBe(false);
  });

  // ── cancelAuth: dismiss the per-op overlay, rejecting parked callers ────────

  it("cancelAuth is a no-op when no per-op auth is in flight (hard lock)", () => {
    s.setLocked(true); // hard lock: authPromise stays null
    expect(() => s.cancelAuth()).not.toThrow();
    expect(s.locked.value).toBe(true);
    expect(s.overlayUp.value).toBe(true); // hard-lock overlay untouched
  });

  it("cancelAuth rejects a parked runWithAuth with AUTH_CANCELLED and drops the overlay", async () => {
    // Per-op auth scenario: identity unlocked (locked=false, cached), then a soft
    // wipe empties the cache without raising the overlay (Immediate post-op).
    vi.mocked(invoke).mockResolvedValue({
      configured: true,
      encrypted: true,
      unlocked: true,
      identity_type: "x25519",
    });
    await s.init();
    expect(s.locked.value).toBe(false);
    expect(s.identityCached.value).toBe(true);

    const handler = vi.mocked(listen).mock.calls[0][1] as (e: {
      payload: { locked: boolean; soft?: boolean };
    }) => void;
    handler({ payload: { locked: true, soft: true } }); // soft wipe: cache empty, overlay down
    expect(s.identityCached.value).toBe(false);
    expect(s.locked.value).toBe(false);

    const op = vi.fn().mockResolvedValue("done");
    const p = s.runWithAuth(op);
    expect(s.overlayUp.value).toBe(true); // authPrompted raised
    expect(op).not.toHaveBeenCalled();

    s.cancelAuth();
    await expect(p).rejects.toMatchObject({ code: AUTH_CANCELLED });
    expect(op).not.toHaveBeenCalled(); // the op never ran
    expect(s.overlayUp.value).toBe(false); // overlay dismissed (locked stayed false)
  });

  it("cancelAuth rejects every concurrent parked caller (single-flight cancel)", async () => {
    vi.mocked(invoke).mockResolvedValue({
      configured: true,
      encrypted: true,
      unlocked: false,
      identity_type: "x25519",
    });
    await s.init();

    const op = vi.fn().mockResolvedValue("done");
    const p1 = s.runWithAuth(op);
    const p2 = s.runWithAuth(op);
    expect(s.overlayUp.value).toBe(true);

    s.cancelAuth();
    await expect(p1).rejects.toMatchObject({ code: AUTH_CANCELLED });
    await expect(p2).rejects.toMatchObject({ code: AUTH_CANCELLED });
    expect(op).not.toHaveBeenCalled();
  });

  it("isAuthCancelled recognizes only the AUTH_CANCELLED code", () => {
    expect(isAuthCancelled({ code: AUTH_CANCELLED })).toBe(true);
    expect(isAuthCancelled({ code: "WRONG_PASSPHRASE" })).toBe(false);
    expect(isAuthCancelled(new Error("boom"))).toBe(false);
    expect(isAuthCancelled(undefined)).toBe(false);
    expect(isAuthCancelled(null)).toBe(false);
  });
});

describe("useLockState() accessor (provide/inject)", () => {
  // The factory (createLockState) holds the logic and is tested directly above;
  // these cover the thin inject wrapper — the part the Vue testing guide says
  // needs a host component (provide/inject needs an active instance).

  it("injects the instance provided under LOCK_KEY", () => {
    const lock = createLockState();
    const [result, app] = withSetup(
      () => useLockState(),
      (a) => a.provide(LOCK_KEY, lock),
    );
    expect(result).toBe(lock);
    app.unmount();
  });

  it("throws (fail-loud) when LOCK_KEY is not provided", () => {
    // Called outside setup so inject() yields undefined and our guard throws —
    // the throw is NOT wrapped by Vue's setup error-handling, so it propagates.
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
    expect(() => useLockState()).toThrow(/LOCK_KEY/);
    warnSpy.mockRestore();
  });
});
