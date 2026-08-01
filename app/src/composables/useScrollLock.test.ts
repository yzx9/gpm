// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  createScrollLockController,
  useScrollLock,
  type ScrollLockController,
} from "./useScrollLock";

describe("createScrollLockController", () => {
  beforeEach(() => {
    // jsdom shares document across tests in a file; start each from a clean
    // inline overflow so a prior test's lock can't bleed in.
    document.documentElement.style.overflow = "";
  });

  it("sets overflow:hidden on acquire and restores it on release", () => {
    const lock = createScrollLockController();
    expect(document.documentElement.style.overflow).toBe("");

    lock.acquire();
    expect(document.documentElement.style.overflow).toBe("hidden");

    lock.release();
    expect(document.documentElement.style.overflow).toBe("");
  });

  it("preserves a pre-existing inline overflow across the lock", () => {
    document.documentElement.style.overflow = "auto";
    const lock = createScrollLockController();

    lock.acquire();
    expect(document.documentElement.style.overflow).toBe("hidden");

    lock.release();
    // Restored to what was there before the lock, not wiped to "".
    expect(document.documentElement.style.overflow).toBe("auto");
  });

  it("stays locked until the last of stacked acquires releases", () => {
    const lock = createScrollLockController();

    lock.acquire();
    lock.acquire();
    expect(document.documentElement.style.overflow).toBe("hidden");

    // Inner shell dismisses first — the outer is still up, so the document
    // stays frozen.
    lock.release();
    expect(document.documentElement.style.overflow).toBe("hidden");

    // Last shell down — now the document unlocks.
    lock.release();
    expect(document.documentElement.style.overflow).toBe("");
  });

  it("release with no matching acquire is a no-op (never goes negative)", () => {
    const lock = createScrollLockController();

    lock.release(); // stray release — must not push count below 0
    expect(document.documentElement.style.overflow).toBe("");

    // A subsequent real acquire/release still works symmetrically.
    lock.acquire();
    expect(document.documentElement.style.overflow).toBe("hidden");
    lock.release();
    expect(document.documentElement.style.overflow).toBe("");
  });
});

describe("useScrollLock", () => {
  // Drive the composable with a fake controller so the test is isolated from
  // the app-wide default singleton (whose count would otherwise accumulate
  // across mounts that don't unmount).
  function mountHost(controller: ScrollLockController) {
    return mount({
      setup() {
        useScrollLock(controller);
        return () => null;
      },
    });
  }

  // `vi.fn()` keeps its mock-matchers on the local refs; the cast only bridges
  // the mock type to `ScrollLockController` at the `useScrollLock` boundary.
  let acquire: ReturnType<typeof vi.fn>;
  let release: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    acquire = vi.fn();
    release = vi.fn();
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it("acquires on mount and releases on unmount", () => {
    const wrapper = mountHost({
      acquire,
      release,
    } as unknown as ScrollLockController);
    expect(acquire).toHaveBeenCalledTimes(1);
    expect(release).not.toHaveBeenCalled();

    wrapper.unmount();
    expect(release).toHaveBeenCalledTimes(1);
    expect(acquire).toHaveBeenCalledTimes(1);
  });

  it("the no-arg default controller locks and restores the document scroller", () => {
    // Exercises the exact path BaseModalShell calls — `useScrollLock()` with no
    // controller → the app-wide default → documentElement — not just a fake.
    // Safe here: the createScrollLockController tests above drive fresh instances,
    // so the default singleton's count is 0 entering this test, and the symmetric
    // mount→unmount returns it to 0.
    expect(document.documentElement.style.overflow).toBe("");

    const wrapper = mount({
      setup() {
        useScrollLock();
        return () => null;
      },
    });
    expect(document.documentElement.style.overflow).toBe("hidden");

    wrapper.unmount();
    expect(document.documentElement.style.overflow).toBe("");
  });
});
