// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { withSetup } from "@/test/withSetup";
import { describe, expect, it, vi } from "vitest";
import { APP_LOCK_KEY, createAppLockStore } from "./useAppLockState";
import { useLockSignals } from "./useLockSignals";
import { createLockState, LOCK_KEY } from "./useLockState";

// Mount useLockSignals().onAnyLock inside a host with both lock stores
// provided, mirroring the app-shell provide block. Returns the combined
// unsubscribe + both stores.
function mountAnyLock(cb: () => void) {
  const lock = createLockState({ unlocked: true });
  const appLock = createAppLockStore();
  const [off, app] = withSetup(
    () => useLockSignals().onAnyLock(cb),
    (a) => {
      a.provide(LOCK_KEY, lock);
      a.provide(APP_LOCK_KEY, appLock);
    },
  );
  return { off, app, lock, appLock };
}

describe("useLockSignals", () => {
  it("fires on an identity hard lock (setLocked true)", () => {
    const cb = vi.fn();
    const { lock, app } = mountAnyLock(cb);
    lock.setLocked(true);
    expect(cb).toHaveBeenCalledTimes(1);
    app.unmount();
  });

  it("fires on a gate re-lock (setAppLocked true)", () => {
    const cb = vi.fn();
    const { appLock, app } = mountAnyLock(cb);
    appLock.setAppLocked(true, "idle");
    expect(cb).toHaveBeenCalledTimes(1);
    app.unmount();
  });

  it("does NOT fire on unlock edges", () => {
    const cb = vi.fn();
    const { lock, appLock, app } = mountAnyLock(cb);
    lock.setLocked(true);
    appLock.setAppLocked(true, "return");
    expect(cb).toHaveBeenCalledTimes(2);
    lock.setLocked(false);
    appLock.setAppLocked(false);
    expect(cb).toHaveBeenCalledTimes(2);
    app.unmount();
  });

  it("the returned unsubscribe removes both subscriptions", () => {
    const cb = vi.fn();
    const { off, lock, appLock, app } = mountAnyLock(cb);
    off();
    lock.setLocked(true);
    appLock.setAppLocked(true, "idle");
    expect(cb).not.toHaveBeenCalled();
    app.unmount();
  });

  it("auto-removes BOTH subscriptions on scope dispose (unmount)", () => {
    // Both halves must go: each underlying registry does its own
    // onScopeDispose. If the gate half leaked, wipers would fire for dead
    // page instances on every later gate re-lock.
    const cb = vi.fn();
    const { lock, appLock, app } = mountAnyLock(cb);
    app.unmount();
    lock.setLocked(true);
    appLock.setAppLocked(true, "idle");
    expect(cb).not.toHaveBeenCalled();
  });
});
