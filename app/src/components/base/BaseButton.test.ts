// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import BaseButton from "./BaseButton.vue";
import BaseSpinner from "./BaseSpinner.vue";

describe("BaseButton", () => {
  it("renders each variant class on the button", () => {
    for (const variant of [
      "primary",
      "secondary",
      "outline",
      "ghost",
      "danger",
      "action",
      "action-danger",
      "link",
    ] as const) {
      const wrapper = mount(BaseButton, { props: { variant } });
      expect(wrapper.find("button").classes(), `variant=${variant}`).toContain(
        variant,
      );
    }
  });

  it("applies the size class — md by default, sm and xs explicit", () => {
    expect(mount(BaseButton).find("button").classes()).toContain("size-md");
    expect(
      mount(BaseButton, { props: { size: "sm" } })
        .find("button")
        .classes(),
    ).toContain("size-sm");
    expect(
      mount(BaseButton, { props: { size: "xs" } })
        .find("button")
        .classes(),
    ).toContain("size-xs");
  });

  it("action variants carry NO size class — they inherit the base 48px min-height (regression guard)", () => {
    // sizeClass returns null for action/action-danger, so they rely entirely on
    // the base `.btn { min-height: 48px }`. If min-height ever moves off `.btn`
    // (or action gains a size class), every action list row collapses below the
    // touch target. Keep min-height on `.btn`; action must stay size-class-free.
    for (const variant of ["action", "action-danger"] as const) {
      const btn = mount(BaseButton, { props: { variant } }).find("button");
      expect(
        btn.classes().some((c) => c.startsWith("size-")),
        `variant=${variant} should have no size class`,
      ).toBe(false);
    }
  });

  it("tone emits a color class only for the link variant", () => {
    expect(
      mount(BaseButton, { props: { variant: "link", tone: "danger" } })
        .find("button")
        .classes(),
    ).toContain("tone-danger");

    // default tone emits no tone class (inherits surrounding color)
    const linkDefault = mount(BaseButton, { props: { variant: "link" } })
      .find("button")
      .classes();
    expect(linkDefault.some((c) => c.startsWith("tone-"))).toBe(false);

    // non-link variants ignore tone entirely (they own their foreground)
    const secondary = mount(BaseButton, {
      props: { variant: "secondary", tone: "danger" },
    })
      .find("button")
      .classes();
    expect(secondary).not.toContain("tone-danger");
  });

  it("loading disables the button and renders a spinner", () => {
    const wrapper = mount(BaseButton, { props: { loading: true } });
    expect((wrapper.find("button").element as HTMLButtonElement).disabled).toBe(
      true,
    );
    expect(wrapper.findComponent(BaseSpinner).exists()).toBe(true);
  });

  it("loading forces disabled even when disabled prop is false", () => {
    const wrapper = mount(BaseButton, {
      props: { loading: true, disabled: false },
    });
    expect((wrapper.find("button").element as HTMLButtonElement).disabled).toBe(
      true,
    );
  });

  it("block adds the block class", () => {
    expect(
      mount(BaseButton, { props: { block: true } })
        .find("button")
        .classes(),
    ).toContain("block");
  });

  it("forwards the native type and defaults to button", () => {
    expect(mount(BaseButton).find("button").attributes("type")).toBe("button");
    expect(
      mount(BaseButton, { props: { type: "submit" } })
        .find("button")
        .attributes("type"),
    ).toBe("submit");
  });
});
