// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { flushPromises } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createBackHandlerRegistry } from "./useBackHandlerRegistry";

// Deferred onBackButtonPress mock so tests control "registration completes" and
// "back pressed". Mirrors useOverlayBackHandler.test.ts / BaseModalShell.test.ts.
// unregister() clears the captured handler so fireBack() after unregister is a
// no-op (mirrors Tauri no longer emitting to a released listener).
const api = vi.hoisted(() => {
  let handler: ((p: { canGoBack: boolean }) => void) | null = null;
  const unregister = vi.fn(async () => {
    handler = null;
  });
  let pendingResolve: ((l: { unregister: typeof unregister }) => void) | null =
    null;
  const onBackButtonPress = vi.fn((h: (p: { canGoBack: boolean }) => void) => {
    handler = h;
    return new Promise<{ unregister: typeof unregister }>((res) => {
      pendingResolve = res;
    });
  });
  const resolveRegistration = () => {
    pendingResolve?.({ unregister });
    pendingResolve = null;
  };
  const fireBack = () => {
    handler?.({ canGoBack: false });
  };
  return { onBackButtonPress, unregister, resolveRegistration, fireBack };
});
vi.mock("@tauri-apps/api/app", () => ({
  onBackButtonPress: api.onBackButtonPress,
}));

describe("createBackHandlerRegistry", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  async function registered() {
    await flushPromises(); // subscribe initiated
    api.resolveRegistration(); // registration completes
    await flushPromises();
  }

  it("a back press fires only the highest-z handler (z-priority)", async () => {
    const reg = createBackHandlerRegistry();
    const a = vi.fn();
    const b = vi.fn();
    reg.push(a, 1000);
    reg.push(b, 2000);
    await registered();

    api.fireBack();
    expect(b).toHaveBeenCalledTimes(1);
    expect(a).not.toHaveBeenCalled();
  });

  it("a same-z tie fires only the most-recently-pushed (LIFO)", async () => {
    const reg = createBackHandlerRegistry();
    const a = vi.fn();
    const b = vi.fn();
    reg.push(a, 1000);
    reg.push(b, 1000);
    await registered();

    api.fireBack();
    expect(b).toHaveBeenCalledTimes(1);
    expect(a).not.toHaveBeenCalled();
  });

  it("dispatch fires exactly one handler with three stacked (broadcast-bug regression)", async () => {
    const reg = createBackHandlerRegistry();
    const fns = [vi.fn(), vi.fn(), vi.fn()];
    for (const f of fns) reg.push(f, 1000);
    await registered();

    api.fireBack();
    expect(fns[2]).toHaveBeenCalledTimes(1); // most-recent wins
    expect(fns[0]).not.toHaveBeenCalled();
    expect(fns[1]).not.toHaveBeenCalled();
  });

  it("pop removes by handle identity, not stack position", async () => {
    const reg = createBackHandlerRegistry();
    const a = vi.fn();
    const b = vi.fn();
    const ha = reg.push(a, 1000); // bottom (older)
    reg.push(b, 1000); // top (most recent, same z)
    reg.pop(ha); // remove the bottom, not the top
    await registered();

    api.fireBack();
    expect(b).toHaveBeenCalledTimes(1);
    expect(a).not.toHaveBeenCalled();
  });

  it("popping the current top lets the next handler receive back", async () => {
    const reg = createBackHandlerRegistry();
    const a = vi.fn();
    const b = vi.fn();
    reg.push(a, 1000);
    const hb = reg.push(b, 1000);
    await registered();

    api.fireBack();
    expect(b).toHaveBeenCalledTimes(1);
    reg.pop(hb);
    api.fireBack();
    expect(a).toHaveBeenCalledTimes(1);
    expect(b).toHaveBeenCalledTimes(1); // unchanged after its single fire
  });

  it("a higher-z handler pushed later beats a lower-z one pushed earlier", async () => {
    const reg = createBackHandlerRegistry();
    const sheet = vi.fn();
    const gate = vi.fn();
    reg.push(sheet, 1000);
    reg.push(gate, 2000); // gate pushed later AND higher z (app-lock-on-resume shape)
    await registered();

    api.fireBack();
    expect(gate).toHaveBeenCalledTimes(1);
    expect(sheet).not.toHaveBeenCalled();
  });

  it("a throwing handler is caught and does not break subsequent dispatch", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    const reg = createBackHandlerRegistry();
    const boom = vi.fn(() => {
      throw new Error("boom");
    });
    reg.push(boom, 1000);
    await registered();

    api.fireBack(); // throws, caught + logged
    expect(boom).toHaveBeenCalledTimes(1);
    expect(console.error).toHaveBeenCalled();

    const next = vi.fn();
    reg.push(next, 2000); // higher z → new top
    api.fireBack();
    expect(next).toHaveBeenCalledTimes(1);
  });

  it("registers one listener on first push; unregisters when the stack drains", async () => {
    const reg = createBackHandlerRegistry();
    const h = reg.push(vi.fn(), 1000);
    await registered();
    expect(api.onBackButtonPress).toHaveBeenCalledTimes(1);
    expect(api.unregister).not.toHaveBeenCalled();

    reg.pop(h);
    await flushPromises();
    expect(api.unregister).toHaveBeenCalledTimes(1);
  });

  it("a second push while a handler is up does not register a second listener", async () => {
    const reg = createBackHandlerRegistry();
    reg.push(vi.fn(), 1000);
    await registered();
    reg.push(vi.fn(), 1000);
    await flushPromises();
    expect(api.onBackButtonPress).toHaveBeenCalledTimes(1);
  });

  it("a registration that resolves after the stack drains is dropped (stale guard)", async () => {
    const reg = createBackHandlerRegistry();
    const h = reg.push(vi.fn(), 1000);
    await flushPromises(); // subscribe pending
    expect(api.onBackButtonPress).toHaveBeenCalledTimes(1);
    reg.pop(h); // drain BEFORE the registration resolves
    await flushPromises();
    api.resolveRegistration(); // stale registration completes now
    await flushPromises();
    expect(api.unregister).toHaveBeenCalledTimes(1); // stale listener dropped
  });

  it("close-then-open during an in-flight unregister never double-fires (release/subscribe race)", async () => {
    const reg = createBackHandlerRegistry();
    const a = vi.fn();
    const b = vi.fn();
    const ha = reg.push(a, 1000);
    await registered();
    expect(api.onBackButtonPress).toHaveBeenCalledTimes(1);

    // Defer the unregister so a push during it can observe the overlap window.
    let resolveUnreg: () => void = () => {};
    api.unregister.mockImplementationOnce(
      () =>
        new Promise<void>((r) => {
          resolveUnreg = r;
        }),
    );

    reg.pop(ha); // release: listener=null, releasing=true, await deferred unregister
    reg.push(b, 1000); // ensure: `releasing` true → defers B's subscribe
    await flushPromises();
    // B is NOT subscribed yet — it waits on the in-flight release. Without the
    // `releasing` guard B would have subscribed already (2nd call) while a is
    // still live → a double-fire window. This assertion fails on the unfixed code.
    expect(api.onBackButtonPress).toHaveBeenCalledTimes(1);

    resolveUnreg(); // the unregister finishes
    await flushPromises(); // release re-triggers ensure → subscribe B
    api.resolveRegistration(); // B's subscribe completes
    await flushPromises();

    expect(api.onBackButtonPress).toHaveBeenCalledTimes(2); // a then b — never both live
    api.fireBack();
    expect(b).toHaveBeenCalledTimes(1); // exactly once, not twice
    expect(a).not.toHaveBeenCalled();
  });
});
