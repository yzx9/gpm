// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createToast, type ToastState } from "./useToast";

describe("useToast", () => {
  let t: ToastState;

  beforeEach(() => {
    vi.useFakeTimers();
    // Fresh per test — no module singleton to reset.
    t = createToast();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("variant methods each set their variant", () => {
    // Fresh host per variant so the 3-cap never evicts the item under test.
    for (const v of ["success", "danger", "info", "warning"] as const) {
      const s = createToast();
      (s.toast[v] as (m: string) => void)("x");
      expect(s.toasts.value[0].variant).toBe(v);
    }
  });

  it("accepts either a plain msg or an opts object", () => {
    t.toast.success("plain");
    t.toast.success({ message: "opts" });
    expect(t.toasts.value.map((x) => x.message)).toEqual(["plain", "opts"]);
  });

  it("auto-dismisses after the default 3000ms", () => {
    t.toast.success("hi");
    expect(t.toasts.value).toHaveLength(1);
    vi.advanceTimersByTime(3000);
    expect(t.toasts.value).toHaveLength(0);
  });

  it("timeout: null is sticky — survives past the default window", () => {
    t.toast.success({ message: "sticky", timeout: null });
    vi.advanceTimersByTime(10_000);
    expect(t.toasts.value.map((x) => x.message)).toEqual(["sticky"]);
  });

  it("timeout is configurable per toast", () => {
    t.toast.success({ message: "quick", timeout: 500 });
    t.toast.success({ message: "long", timeout: 5000 });
    vi.advanceTimersByTime(500); // "quick" dismissed, "long" remains
    expect(t.toasts.value.map((x) => x.message)).toEqual(["long"]);
  });

  it("closable defaults to true when sticky or > 5000ms, else false", () => {
    // One toast per fresh host (the 3-cap would otherwise evict earlier cases).
    const check = (opts: { timeout?: number | null }, expected: boolean) => {
      const s = createToast();
      s.toast.success({ message: "x", ...opts });
      expect(s.toasts.value[0].closable).toBe(expected);
    };
    check({}, false); // default 3000 → false
    check({ timeout: null }, true); // sticky → true
    check({ timeout: 6000 }, true); // > 5000 → true
    check({ timeout: 5000 }, false); // not > 5000 → false
  });

  it("closable can be overridden explicitly", () => {
    t.toast.success({ message: "x", closable: true });
    t.toast.success({ message: "sticky", timeout: null, closable: false });
    expect(t.toasts.value.map((x) => x.closable)).toEqual([true, false]);
  });

  it("show() defaults to info and honors an explicit variant", () => {
    t.toast.show({ message: "neutral" });
    t.toast.show({ message: "bad", variant: "danger" });
    expect(t.toasts.value.map((x) => x.variant)).toEqual(["info", "danger"]);
  });

  it("a push returns a dismiss fn that removes just that toast", () => {
    const hide = t.toast.success("first");
    t.toast.success("second");
    expect(t.toasts.value).toHaveLength(2);
    hide();
    expect(t.toasts.value.map((x) => x.message)).toEqual(["second"]);
  });

  it("dismiss(id) removes the toast with that id (host ×-button path)", () => {
    t.toast.success("a");
    const id = t.toasts.value[0].id;
    t.toast.dismiss(id);
    expect(t.toasts.value).toHaveLength(0);
  });

  it("caps the queue at 3, dropping the oldest", () => {
    t.toast.success("a");
    t.toast.success("b");
    t.toast.success("c");
    t.toast.success("d"); // over the cap → "a" (oldest) dropped
    expect(t.toasts.value.map((x) => x.message)).toEqual(["b", "c", "d"]);
  });

  it("cap-eviction clears the evicted toast's timer, not a survivor's", () => {
    t.toast.success("a");
    t.toast.success("b");
    t.toast.success("c");
    t.toast.success("d"); // evicts "a" and must clear exactly a's timer
    expect(t.toasts.value.map((x) => x.message)).toEqual(["b", "c", "d"]);
    vi.advanceTimersByTime(3000); // b/c/d auto-dismiss; a's timer was cleared
    expect(t.toasts.value).toHaveLength(0);
  });

  it("dismiss is a no-op for unknown ids and for already-removed toasts", () => {
    t.toast.success("keep");
    t.toast.dismiss(t.toasts.value[0]!.id + 999); // unknown id
    expect(t.toasts.value).toHaveLength(1);
    const hide = t.toast.success("ephemeral");
    hide();
    expect(t.toasts.value.map((x) => x.message)).toEqual(["keep"]);
    hide(); // already removed — second call is a no-op
    expect(t.toasts.value.map((x) => x.message)).toEqual(["keep"]);
  });

  it("a dismiss fn whose toast was cap-evicted does not remove a survivor", () => {
    const hideA = t.toast.success("a");
    t.toast.success("b");
    t.toast.success("c");
    t.toast.success("d"); // evicts "a"
    expect(t.toasts.value.map((x) => x.message)).toEqual(["b", "c", "d"]);
    hideA(); // "a" already gone — must not splice out "b"
    expect(t.toasts.value.map((x) => x.message)).toEqual(["b", "c", "d"]);
  });

  it("auto-dismisses at exactly the timeout boundary, not before", () => {
    t.toast.success("hi");
    vi.advanceTimersByTime(2999);
    expect(t.toasts.value).toHaveLength(1);
    vi.advanceTimersByTime(1); // total 3000 → timer fires
    expect(t.toasts.value).toHaveLength(0);
  });
});
