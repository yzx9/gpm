// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import BaseModalShell from "./BaseModalShell.vue";

describe("BaseModalShell", () => {
  it("emits `close` when the overlay backdrop is clicked", async () => {
    const wrapper = mount(BaseModalShell, { props: { variant: "center" } });
    await wrapper.find(".overlay").trigger("click");
    expect(wrapper.emitted("close")).toHaveLength(1);
  });

  it("does NOT emit `close` when a click lands inside the card", async () => {
    const wrapper = mount(BaseModalShell, {
      props: { variant: "center" },
      slots: { default: "<p>body</p>" },
    });
    // .wrap sits between the backdrop and the card; a click there bubbles to
    // .overlay but is not `.self`, so it must not close.
    await wrapper.find(".wrap").trigger("click");
    expect(wrapper.emitted("close")).toBeUndefined();
  });

  it("defaults z-index to 60 for `center` and 40 for `sheet`", () => {
    const center = mount(BaseModalShell, { props: { variant: "center" } });
    expect(center.find(".overlay").attributes("style")).toContain(
      "z-index: 60",
    );

    const sheet = mount(BaseModalShell, { props: { variant: "sheet" } });
    expect(sheet.find(".overlay").attributes("style")).toContain("z-index: 40");
  });

  it("honors an explicit `z` override (app-lock sits above the identity modal)", () => {
    const wrapper = mount(BaseModalShell, {
      props: { variant: "center", z: 70 },
    });
    expect(wrapper.find(".overlay").attributes("style")).toContain(
      "z-index: 70",
    );
  });
});
