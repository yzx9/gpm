// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { withSetup } from "@/test/withSetup";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createLockState, LOCK_KEY } from "./useLockState";
import { useWipeOnLeave } from "./useWipeOnLeave";

// Mount `useWipeOnLeave` in a throwaway host with a fresh, unlocked lock state
// provided under LOCK_KEY. Returns the wipe spy, the lock, and the app (call
// `app.unmount()` to fire the onBeforeUnmount wipe). vue-router is mocked in
// setup.ts, so popstate must be driven explicitly via window.dispatchEvent —
// programmatic router.back() does not touch real history.
function mountWipe(opts?: { lock?: boolean }) {
  const wipe = vi.fn();
  const lock = createLockState({ unlocked: true });
  const [, app] = withSetup(
    () => useWipeOnLeave(wipe, opts),
    (a) => a.provide(LOCK_KEY, lock),
  );
  return { wipe, lock, app };
}

const firePopstate = () => window.dispatchEvent(new PopStateEvent("popstate"));

describe("useWipeOnLeave", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("fires wipe on popstate (browser/Android back)", () => {
    const { wipe, app } = mountWipe();
    expect(wipe).not.toHaveBeenCalled();
    firePopstate();
    expect(wipe).toHaveBeenCalledTimes(1);
    app.unmount();
  });

  it("fires wipe on unmount and removes the popstate listener", () => {
    const { wipe, app } = mountWipe();
    app.unmount();
    expect(wipe).toHaveBeenCalledTimes(1); // the unmount wipe
    // The listener is gone after unmount — a later popstate must not re-fire.
    firePopstate();
    expect(wipe).toHaveBeenCalledTimes(1);
  });

  it("fires wipe on a hard identity lock via onLock (lock default true)", () => {
    const { wipe, lock, app } = mountWipe();
    lock.setLocked(false); // unlocking fires nothing
    expect(wipe).not.toHaveBeenCalled();
    lock.setLocked(true); // hard lock → onLock fires
    expect(wipe).toHaveBeenCalledTimes(1);
    app.unmount();
  });

  it("does NOT fire wipe on a soft wipe — onLock is hard-lock only (inherited from useLockState)", async () => {
    // Default lock state is not yet initialized, so init() subscribes the
    // identity-lock-state listener whose handler we drive below.
    const wipe = vi.fn();
    const lock = createLockState();
    const [, app] = withSetup(
      () => useWipeOnLeave(wipe),
      (a) => a.provide(LOCK_KEY, lock),
    );
    vi.mocked(invoke).mockResolvedValue({
      configured: true,
      encrypted: true,
      unlocked: true,
      identity_type: "x25519",
    });
    await lock.init();

    const handler = vi.mocked(listen).mock.calls[0][1] as (e: {
      payload: { locked: boolean; soft?: boolean };
    }) => void;
    // Soft wipe: identity leaves the cache, but onLock must NOT fire (a revealed
    // secret / draft survives it).
    handler({ payload: { locked: true, soft: true } });
    expect(wipe).not.toHaveBeenCalled();
    app.unmount();
  });

  it("lock: false skips onLock while popstate + unmount still fire", () => {
    const { wipe, lock, app } = mountWipe({ lock: false });
    lock.setLocked(true); // would fire if onLock were wired
    expect(wipe).not.toHaveBeenCalled();
    firePopstate();
    expect(wipe).toHaveBeenCalledTimes(1);
    app.unmount();
    expect(wipe).toHaveBeenCalledTimes(2); // popstate + unmount
  });

  it("wipe may fire twice in one back navigation (popstate then unmount) — idempotent contract", () => {
    const { wipe, app } = mountWipe();
    firePopstate();
    app.unmount();
    // No throw; real sites must keep wipe safe to call repeatedly.
    expect(wipe).toHaveBeenCalledTimes(2);
  });
});
