// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { Z } from "@/zTiers";
import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import { defineComponent } from "vue";
import { DIALOG_KEY, createDialog, useDialog } from "./useDialog";

describe("useDialog", () => {
  it("confirm() enqueues a confirm request carrying the opts", () => {
    const d = createDialog();
    void d.dialog.confirm({
      message: "sure?",
      confirmLabel: "Do it",
      danger: true,
    });
    expect(d.pending.value).toHaveLength(1);
    const req = d.pending.value[0]!;
    expect(req.kind).toBe("confirm");
    expect(req.opts.message).toBe("sure?");
    expect(req.opts.confirmLabel).toBe("Do it");
    expect(req.opts.danger).toBe(true);
  });

  it("confirm() carries an optional `z` stacking-tier override", () => {
    // The lock screen passes Z.gate so its confirm stacks above its own opaque
    // surface; undefined (the default) leaves every other caller at Z.overlay.
    const d = createDialog();
    void d.dialog.confirm({ message: "m", z: Z.gate });
    expect(d.pending.value[0]!.opts.z).toBe(Z.gate);

    const e = createDialog();
    void e.dialog.confirm({ message: "m" });
    expect(e.pending.value[0]!.opts.z).toBeUndefined();
  });

  it("assigns monotonic ids per host instance", () => {
    const d = createDialog();
    void d.dialog.confirm({ message: "a" });
    void d.dialog.confirm({ message: "b" });
    expect(d.pending.value.map((r) => r.id)).toEqual([0, 1]);
  });

  it("resolve(true) awaits true and removes the request from the queue", async () => {
    const d = createDialog();
    const p = d.dialog.confirm({ message: "m" });
    d.pending.value[0]!.resolve(true);
    expect(await p).toBe(true);
    expect(d.pending.value).toHaveLength(0);
  });

  it("resolve(false) awaits false and removes the request", async () => {
    const d = createDialog();
    const p = d.dialog.confirm({ message: "m" });
    d.pending.value[0]!.resolve(false);
    expect(await p).toBe(false);
    expect(d.pending.value).toHaveLength(0);
  });

  it("two queued confirms resolve independently — resolving one never touches the other", async () => {
    const d = createDialog();
    const p1 = d.dialog.confirm({ message: "first" });
    const p2 = d.dialog.confirm({ message: "second" });
    expect(d.pending.value).toHaveLength(2);

    // Resolve the SECOND first — the first must stay pending, untouched.
    d.pending.value[1]!.resolve(true);
    expect(await p2).toBe(true);
    expect(d.pending.value).toHaveLength(1);
    expect(d.pending.value[0]!.opts.message).toBe("first");

    d.pending.value[0]!.resolve(false);
    expect(await p1).toBe(false);
    expect(d.pending.value).toHaveLength(0);
  });

  it("resolve is idempotent — a second call on a settled request is a no-op", async () => {
    const d = createDialog();
    const p = d.dialog.confirm({ message: "m" });
    const req = d.pending.value[0]!;
    req.resolve(true);
    expect(await p).toBe(true);
    expect(d.pending.value).toHaveLength(0);
    // Already removed — must not throw or re-resolve.
    expect(() => req.resolve(false)).not.toThrow();
    expect(d.pending.value).toHaveLength(0);
  });

  it("useDialog() throws when DIALOG_KEY is not provided", () => {
    // Mount a component whose setup calls useDialog with no provider, so the
    // throw surfaces the way it would in a real mis-provisioned app.
    const Bad = defineComponent({
      setup() {
        useDialog();
        return {};
      },
    });
    expect(() => mount(Bad)).toThrow(/DIALOG_KEY/);
  });

  it("useDialog() returns the provided state under DIALOG_KEY", () => {
    const d = createDialog();
    const Good = defineComponent({
      setup() {
        const s = useDialog();
        return { s };
      },
    });
    const wrapper = mount(Good, {
      global: { provide: { [DIALOG_KEY]: d } },
    });
    expect(wrapper.vm.s).toBe(d);
    wrapper.unmount();
  });
});
