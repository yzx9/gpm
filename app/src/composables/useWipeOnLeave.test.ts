// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { withSetup } from "@/test/withSetup";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { APP_LOCK_KEY, createAppLockStore } from "./useAppLockState";
import { createDraftsNotice, DRAFTS_NOTICE_KEY } from "./useDraftsNotice";
import { createLockState, LOCK_KEY } from "./useLockState";
import { useWipeOnLeave } from "./useWipeOnLeave";

// Mount `useWipeOnLeave` in a throwaway host with the full lock provide block
// (identity + gate + drafts notice). Returns the wipe spy, both lock stores,
// the notice, and the app (call `app.unmount()` to fire the onBeforeUnmount
// wipe). vue-router is mocked in setup.ts, so popstate must be driven
// explicitly via window.dispatchEvent — programmatic router.back() does not
// touch real history.
function mountWipe(
  opts?: { lock?: boolean },
  wipe: () => boolean | void = vi.fn(),
) {
  const lock = createLockState({ unlocked: true });
  const appLock = createAppLockStore();
  const notice = createDraftsNotice();
  const [, app] = withSetup(
    () => useWipeOnLeave(wipe, opts),
    (a) => {
      a.provide(LOCK_KEY, lock);
      a.provide(APP_LOCK_KEY, appLock);
      a.provide(DRAFTS_NOTICE_KEY, notice);
    },
  );
  return { wipe, lock, appLock, notice, app };
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

  it("fires wipe on a gate re-lock via onAppLock (the mask does not unmount the page)", () => {
    const { wipe, appLock, app } = mountWipe();
    appLock.setAppLocked(true, "idle"); // gate lock edge → onAppLock fires
    expect(wipe).toHaveBeenCalledTimes(1);
    appLock.setAppLocked(true, "return"); // locked→locked: no re-fire
    expect(wipe).toHaveBeenCalledTimes(1);
    app.unmount();
  });

  it("marks the drafts notice when the gate wipe returns true, not when it returns void", () => {
    const wipe = vi.fn(() => true);
    const { appLock, notice, app } = mountWipe(undefined, wipe);
    appLock.setAppLocked(true, "idle");
    expect(notice.consume()).toBe(true); // marked; consumed here to reset
    app.unmount();

    const quiet = vi.fn(() => undefined);
    const m2 = mountWipe(undefined, quiet);
    m2.appLock.setAppLocked(true, "idle");
    expect(m2.notice.consume()).toBe(false); // void return → no mark
    m2.app.unmount();
  });

  it("marks the drafts notice on the identity hard-lock path too (both lock paths)", () => {
    const wipe = vi.fn(() => true);
    const { lock, notice, app } = mountWipe(undefined, wipe);
    lock.setLocked(true);
    expect(notice.consume()).toBe(true);
    app.unmount();
  });

  it("a true-returning wipe on popstate/unmount does NOT mark the notice (lock-only contract)", () => {
    // The return value is documented as lock-only: a back-navigation away
    // from a dirty draft must not produce a spurious "drafts cleared" toast
    // at the next unlock edge.
    const wipe = vi.fn(() => true);
    const { notice, app } = mountWipe(undefined, wipe);
    firePopstate();
    app.unmount();
    expect(wipe).toHaveBeenCalledTimes(2);
    expect(notice.consume()).toBe(false);
  });

  it("does NOT fire wipe on a soft wipe — onLock is hard-lock only (inherited from useLockState)", async () => {
    // Default lock state is not yet initialized, so init() subscribes the
    // identity-lock-state listener whose handler we drive below.
    const wipe = vi.fn();
    const lock = createLockState();
    const appLock = createAppLockStore();
    const notice = createDraftsNotice();
    const [, app] = withSetup(
      () => useWipeOnLeave(wipe),
      (a) => {
        a.provide(LOCK_KEY, lock);
        a.provide(APP_LOCK_KEY, appLock);
        a.provide(DRAFTS_NOTICE_KEY, notice);
      },
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

  it("lock: false skips BOTH lock signals while popstate + unmount still fire", () => {
    const { wipe, lock, appLock, app } = mountWipe({ lock: false });
    lock.setLocked(true); // would fire if onLock were wired
    appLock.setAppLocked(true, "idle"); // would fire if onAppLock were wired
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
