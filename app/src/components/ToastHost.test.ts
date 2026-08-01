// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import BaseAlert from "@/components/base/BaseAlert.vue";
import { createToast, TOAST_KEY } from "@/composables";
import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ToastHost from "./ToastHost.vue";

function mountHost() {
  const t = createToast();
  const wrapper = mount(ToastHost, {
    global: { provide: { [TOAST_KEY]: t } },
  });
  return { wrapper, t };
}

describe("ToastHost", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("renders nothing when the queue is empty", () => {
    const { wrapper } = mountHost();
    expect(wrapper.text()).toBe("");
    expect(wrapper.findAllComponents(BaseAlert)).toHaveLength(0);
  });

  it("renders one BaseAlert per toast, bound to its variant + message", async () => {
    const { wrapper, t } = mountHost();
    t.toast.success("ok");
    t.toast.danger("boom");
    await flushPromises();

    const alerts = wrapper.findAllComponents(BaseAlert);
    expect(alerts).toHaveLength(2);
    expect(alerts[0]!.props("variant")).toBe("success");
    expect(alerts[0]!.text()).toContain("ok");
    expect(alerts[1]!.props("variant")).toBe("danger");
    expect(alerts[1]!.text()).toContain("boom");
  });

  it("shows a close button only when the item is closable", async () => {
    const { wrapper, t } = mountHost();
    t.toast.success("transient"); // default 3000 → closable false
    t.toast.success({ message: "sticky", timeout: null }); // → closable true
    await flushPromises();

    expect(wrapper.findAll('button[aria-label="Close"]')).toHaveLength(1);
  });

  it("clicking × dismisses only that toast", async () => {
    const { wrapper, t } = mountHost();
    t.toast.success("keep");
    t.toast.success({ message: "kill", timeout: null });
    await flushPromises();
    expect(wrapper.findAllComponents(BaseAlert)).toHaveLength(2);

    await wrapper.find('button[aria-label="Close"]').trigger("click");
    await flushPromises();

    const alerts = wrapper.findAllComponents(BaseAlert);
    expect(alerts).toHaveLength(1);
    expect(alerts[0]!.text()).toContain("keep");
  });
});
