// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";
import type { Router } from "vue-router";
import { navBack } from "./nav";

function makeRouter(): Router {
  return { back: vi.fn(), replace: vi.fn() } as unknown as Router;
}

const originalState = window.history.state;

afterEach(() => {
  // jsdom: restore the real history.state after each override.
  Object.defineProperty(window.history, "state", {
    value: originalState,
    configurable: true,
    writable: true,
  });
});

function setState(state: { position: number } | null): void {
  Object.defineProperty(window.history, "state", {
    value: state,
    configurable: true,
    writable: true,
  });
}

describe("navBack", () => {
  it("pops via router.back when a previous entry exists (position > 0)", () => {
    setState({ position: 2 });
    const router = makeRouter();

    navBack(router, { name: "entries" });

    expect(router.back).toHaveBeenCalledTimes(1);
    expect(router.replace).not.toHaveBeenCalled();
  });

  it("replaces to the fallback at a deep-link root (position === 0)", () => {
    setState({ position: 0 });
    const router = makeRouter();

    navBack(router, { name: "entries" });

    expect(router.replace).toHaveBeenCalledWith({ name: "entries" });
    expect(router.back).not.toHaveBeenCalled();
  });

  it("falls back to replace when history.state is missing (defensive)", () => {
    setState(null);
    const router = makeRouter();

    navBack(router, { name: "entries" });

    expect(router.replace).toHaveBeenCalledWith({ name: "entries" });
    expect(router.back).not.toHaveBeenCalled();
  });
});
