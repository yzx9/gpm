// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  createScrollLockController,
  SCROLL_LOCK_KEY,
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
  // Drive the composable with a fake controller provided under SCROLL_LOCK_KEY,
  // so the test is isolated from any real controller's count.
  function mountHost(controller: ScrollLockController) {
    return mount(
      {
        setup() {
          useScrollLock();
          return () => null;
        },
      },
      {
        global: { provide: { [SCROLL_LOCK_KEY]: controller } },
      },
    );
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

  it("throws when SCROLL_LOCK_KEY is not provided", () => {
    // Mount a component whose setup calls useScrollLock with no provider, so the
    // throw surfaces the way it would in a real mis-provisioned app.
    const Bad = {
      setup() {
        useScrollLock();
        return () => null;
      },
    };
    expect(() => mount(Bad)).toThrow(/SCROLL_LOCK_KEY/);
  });

  it("the provided controller locks and restores the document scroller", () => {
    // Exercises the exact path BaseModalShell takes — `useScrollLock()` injects
    // the provided controller → documentElement — not just a fake. A fresh
    // controller starts at count 0, and the symmetric mount→unmount returns it
    // to 0.
    document.documentElement.style.overflow = "";
    expect(document.documentElement.style.overflow).toBe("");

    const wrapper = mountHost(createScrollLockController());
    expect(document.documentElement.style.overflow).toBe("hidden");

    wrapper.unmount();
    expect(document.documentElement.style.overflow).toBe("");
  });

  it("shares one injected controller across stacked mounts — locked until the last unmount", () => {
    // The production scenario the ref-count exists for: two shells up at once
    // (e.g. a page modal with the identity UnlockModal stacked above it) share
    // ONE injected controller, so the document stays frozen until the LAST shell
    // unmounts. mountHost gives each mount its own app but the same controller
    // instance, so acquire/release hit one count.
    document.documentElement.style.overflow = "";

    const shared = createScrollLockController();
    const outer = mountHost(shared);
    const inner = mountHost(shared);
    expect(document.documentElement.style.overflow).toBe("hidden");

    // Inner shell down first — outer still up, document stays frozen.
    inner.unmount();
    expect(document.documentElement.style.overflow).toBe("hidden");

    // Last shell down — document unlocks.
    outer.unmount();
    expect(document.documentElement.style.overflow).toBe("");
  });
});
