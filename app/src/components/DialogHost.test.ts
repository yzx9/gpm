// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import BaseButton from "@/components/base/BaseButton.vue";
import { DIALOG_KEY, createDialog } from "@/composables";
import { enableAutoUnmount, flushPromises, mount } from "@vue/test-utils";
import { afterEach, describe, expect, it } from "vitest";
import DialogHost from "./DialogHost.vue";

// DialogHost mounts BaseModalShell per pending request, and BaseModalShell
// locks the document scroller on mount. Unmount every wrapper after each test
// so the shared scroll-lock count returns to 0 (mirrors BaseModalShell.test.ts).
enableAutoUnmount(afterEach);

function mountHost() {
  const d = createDialog();
  const wrapper = mount(DialogHost, {
    global: { provide: { [DIALOG_KEY]: d } },
  });
  return { wrapper, d };
}

describe("DialogHost", () => {
  it("renders nothing when the queue is empty", () => {
    const { wrapper } = mountHost();
    expect(wrapper.text()).toBe("");
  });

  it("renders the message + a confirm and a cancel button for a pending confirm", async () => {
    const { wrapper, d } = mountHost();
    void d.dialog.confirm({
      message: "Delete this?",
      confirmLabel: "Delete",
      cancelLabel: "Cancel",
    });
    await flushPromises();

    expect(wrapper.text()).toContain("Delete this?");
    const btns = wrapper.findAllComponents(BaseButton);
    expect(btns).toHaveLength(2);
    expect(btns[0]!.text()).toContain("Delete");
    expect(btns[1]!.text()).toContain("Cancel");
  });

  it("clicking the confirm button resolves true and clears the queue", async () => {
    const { wrapper, d } = mountHost();
    const p = d.dialog.confirm({
      message: "m",
      confirmLabel: "OK",
      cancelLabel: "No",
    });
    await flushPromises();

    await wrapper.findAllComponents(BaseButton)[0]!.trigger("click");
    await flushPromises();

    expect(await p).toBe(true);
    expect(d.pending.value).toHaveLength(0);
  });

  it("clicking Cancel resolves false", async () => {
    const { wrapper, d } = mountHost();
    const p = d.dialog.confirm({
      message: "m",
      confirmLabel: "OK",
      cancelLabel: "No",
    });
    await flushPromises();

    await wrapper.findAllComponents(BaseButton)[1]!.trigger("click");
    await flushPromises();

    expect(await p).toBe(false);
  });

  it("a backdrop tap resolves false (BaseModalShell @close → resolve(false))", async () => {
    const { wrapper, d } = mountHost();
    const p = d.dialog.confirm({
      message: "m",
      confirmLabel: "OK",
      cancelLabel: "No",
    });
    await flushPromises();

    await wrapper.find(".overlay").trigger("click");
    await flushPromises();

    expect(await p).toBe(false);
  });

  it("styles the confirm button as danger when danger:true (cancel stays outline)", async () => {
    const { wrapper, d } = mountHost();
    void d.dialog.confirm({
      message: "m",
      confirmLabel: "Delete",
      danger: true,
    });
    await flushPromises();

    const btns = wrapper.findAllComponents(BaseButton);
    expect(btns[0]!.props("variant")).toBe("danger");
    expect(btns[1]!.props("variant")).toBe("outline");
  });

  it("styles the confirm button as primary when danger is unset", async () => {
    const { wrapper, d } = mountHost();
    void d.dialog.confirm({ message: "m", confirmLabel: "Export" });
    await flushPromises();

    expect(wrapper.findAllComponents(BaseButton)[0]!.props("variant")).toBe(
      "primary",
    );
  });
});
