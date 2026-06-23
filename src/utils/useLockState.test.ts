// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect, vi, beforeEach } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  __resetLockStateForTests,
  __unlockForTests,
  onLock,
  runWithAuth,
  useLockState,
} from "./useLockState";

describe("useLockState", () => {
  const { locked, overlayUp, identityCached, ready, init, setLocked } =
    useLockState();

  beforeEach(() => {
    vi.clearAllMocks();
    __resetLockStateForTests();
  });

  it("is fail-closed (locked) and not ready until init() resolves", () => {
    expect(locked.value).toBe(true);
    expect(ready.value).toBe(false);
  });

  it("init() reflects encrypted+locked auth state and flips ready", async () => {
    vi.mocked(invoke).mockResolvedValue({
      configured: true,
      encrypted: true,
      unlocked: false,
      identity_type: "x25519",
    });

    await init();

    expect(invoke).toHaveBeenCalledWith("get_auth_state");
    expect(locked.value).toBe(true);
    expect(ready.value).toBe(true);
  });

  it("init() unlocks for an encrypted-but-already-unlocked identity", async () => {
    vi.mocked(invoke).mockResolvedValue({
      configured: true,
      encrypted: true,
      unlocked: true,
      identity_type: "x25519",
    });

    await init();

    expect(locked.value).toBe(false);
  });

  it("init() unlocks for an unencrypted identity (nothing to lock)", async () => {
    vi.mocked(invoke).mockResolvedValue({
      configured: true,
      encrypted: false,
      unlocked: false,
      identity_type: "x25519",
    });

    await init();

    expect(locked.value).toBe(false);
  });

  it("init() stays fail-closed when get_auth_state rejects", async () => {
    vi.mocked(invoke).mockRejectedValue(new Error("boom"));

    await init();

    expect(locked.value).toBe(true);
  });

  it("init() registers a single identity-lock-state listener and is idempotent", async () => {
    vi.mocked(invoke).mockResolvedValue({
      configured: true,
      encrypted: true,
      unlocked: true,
      identity_type: "x25519",
    });

    await init();
    await init();

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
    await init();
    expect(locked.value).toBe(false);

    const handler = vi.mocked(listen).mock.calls[0][1] as (e: {
      payload: { locked: boolean };
    }) => void;

    // Backend says locked → mirror it (and fire clear-on-lock).
    handler({ payload: { locked: true } });
    expect(locked.value).toBe(true);

    // Backend says unlocked → mirror it.
    handler({ payload: { locked: false } });
    expect(locked.value).toBe(false);
  });

  it("setLocked(true) fires onLock callbacks AFTER flipping the ref", () => {
    const seen: boolean[] = [];
    onLock(() => seen.push(locked.value));

    setLocked(false);
    expect(seen).toEqual([]); // unlocking fires nothing

    setLocked(true);
    expect(seen).toEqual([true]); // fired, and saw the already-flipped ref
  });

  it("setLocked with the same value is a no-op (re-lock while open)", () => {
    const cb = vi.fn();
    onLock(cb);

    setLocked(true); // default was already true → no transition
    expect(cb).not.toHaveBeenCalled();
  });

  it("a throwing clearer does not block subsequent clearers", () => {
    const second = vi.fn();
    onLock(() => {
      throw new Error("boom");
    });
    onLock(second);

    setLocked(false);
    setLocked(true);

    expect(second).toHaveBeenCalledTimes(1);
  });

  it("onLock returns an unsubscribe that removes the callback", () => {
    const cb = vi.fn();
    const off = onLock(cb);

    setLocked(false);
    off();
    setLocked(true);

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
    await init();
    expect(identityCached.value).toBe(true);
    expect(locked.value).toBe(false);

    const clearer = vi.fn();
    onLock(clearer);

    const handler = vi.mocked(listen).mock.calls[0][1] as (e: {
      payload: { locked: boolean; soft?: boolean };
    }) => void;
    // Soft wipe: the identity leaves the cache, but the overlay stays down and
    // onLock must NOT fire (a revealed secret / draft survives it).
    handler({ payload: { locked: true, soft: true } });
    expect(identityCached.value).toBe(false);
    expect(locked.value).toBe(false);
    expect(overlayUp.value).toBe(false);
    expect(clearer).not.toHaveBeenCalled();
  });

  it("runWithAuth runs the op immediately when the identity is cached", async () => {
    __unlockForTests();
    const op = vi.fn().mockResolvedValue("done");
    await expect(runWithAuth(op)).resolves.toBe("done");
    expect(op).toHaveBeenCalledTimes(1);
    expect(overlayUp.value).toBe(false);
  });

  it("runWithAuth prompts, then resumes after an unlock when not cached", async () => {
    vi.mocked(invoke).mockResolvedValue({
      configured: true,
      encrypted: true,
      unlocked: false,
      identity_type: "x25519",
    });
    await init();
    expect(identityCached.value).toBe(false);

    const op = vi.fn().mockResolvedValue("done");
    const p = runWithAuth(op);
    // Blocked: the per-op overlay is raised and the op has not run.
    expect(overlayUp.value).toBe(true);
    expect(op).not.toHaveBeenCalled();

    const handler = vi.mocked(listen).mock.calls[0][1] as (e: {
      payload: { locked: boolean; soft?: boolean };
    }) => void;
    handler({ payload: { locked: false } }); // backend reports unlock
    await expect(p).resolves.toBe("done");
    expect(op).toHaveBeenCalledTimes(1);
    expect(identityCached.value).toBe(true);
    expect(overlayUp.value).toBe(false);
  });
});
