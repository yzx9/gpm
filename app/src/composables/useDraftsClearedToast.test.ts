// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { useDraftsClearedToast } from "@/composables/useDraftsClearedToast";
import { mountWithApp } from "@/test/appTestUtils";
import { enableAutoUnmount, flushPromises } from "@vue/test-utils";
import { afterEach, describe, expect, it } from "vitest";
import { defineComponent } from "vue";

// The composable needs i18n + all four injections, so mount it in a host via
// mountWithApp (which provides the full app-shell block and returns every
// store instance for driving). Both stores start unlocked; each test drives
// explicit lock→unlock edges.
enableAutoUnmount(afterEach);

function mountToast() {
  const Host = defineComponent({
    setup() {
      useDraftsClearedToast();
      return () => null;
    },
  });
  return mountWithApp(Host);
}

describe("useDraftsClearedToast", () => {
  it("marked notice + gate unlock edge → one toast, notice consumed", async () => {
    const m = mountToast();
    m.draftsNotice.mark();
    m.appLock.setAppLocked(true, "idle"); // lock (wipes drafts in production)
    await flushPromises();
    expect(m.toast.toasts.value.length).toBe(0); // nothing before the unlock

    m.appLock.setAppLocked(false); // unlock edge → toast
    await flushPromises();
    expect(m.toast.toasts.value.length).toBe(1);
    // Sticky with a × button — the toast lands while the user is busy with
    // the unlock prompt, so a 3s transient would die unread.
    expect(m.toast.toasts.value[0].closable).toBe(true);
    expect(m.draftsNotice.consume()).toBe(false); // consumed by the toast
  });

  it("no mark + gate unlock edge → no toast (nothing was lost)", async () => {
    const m = mountToast();
    m.appLock.setAppLocked(true, "idle");
    m.appLock.setAppLocked(false);
    await flushPromises();
    expect(m.toast.toasts.value.length).toBe(0);
  });

  it("marked notice + identity unlock edge → one toast", async () => {
    const m = mountToast();
    m.draftsNotice.mark();
    m.lock.setLocked(true);
    await flushPromises(); // let the lock flip flush before unlocking
    m.lock.setLocked(false); // identity unlock edge
    await flushPromises();
    expect(m.toast.toasts.value.length).toBe(1);
  });

  it("one mark, both locks unlocking in the same cycle → exactly one toast", async () => {
    const m = mountToast();
    m.draftsNotice.mark();
    m.lock.setLocked(true);
    m.appLock.setAppLocked(true, "return");
    await flushPromises();
    m.lock.setLocked(false);
    m.appLock.setAppLocked(false);
    await flushPromises();
    expect(m.toast.toasts.value.length).toBe(1);
  });

  it("a second lock/unlock cycle with no new mark → no second toast", async () => {
    const m = mountToast();
    m.draftsNotice.mark();
    m.appLock.setAppLocked(true, "idle");
    await flushPromises(); // let the lock flip flush before unlocking
    m.appLock.setAppLocked(false);
    await flushPromises();
    expect(m.toast.toasts.value.length).toBe(1);
    m.appLock.setAppLocked(true, "return");
    await flushPromises();
    m.appLock.setAppLocked(false);
    await flushPromises();
    expect(m.toast.toasts.value.length).toBe(1);
  });
});
