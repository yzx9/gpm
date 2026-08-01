// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import BaseSegmentedControl from "./BaseSegmentedControl.vue";

describe("BaseSegmentedControl", () => {
  it("marks the matching option active via `===` by default and emits on select", async () => {
    const wrapper = mount(BaseSegmentedControl, {
      props: {
        name: "view-clear",
        modelValue: 180,
        options: [
          { label: "10s", value: 10 },
          { label: "3 min", value: 180 },
        ],
      },
    });
    const radios = wrapper.findAll('input[type="radio"]');
    expect((radios[0]!.element as HTMLInputElement).checked).toBe(false);
    expect((radios[1]!.element as HTMLInputElement).checked).toBe(true);

    await radios[0]!.trigger("change");
    expect(wrapper.emitted("change")![0]).toEqual([10]);
  });

  it("uses the `by` comparator for object-valued options (auto-lock presets)", async () => {
    type Mode = { idle: number };
    const opts: { label: string; value: Mode }[] = [
      { label: "1 min", value: { idle: 60 } },
      { label: "5 min", value: { idle: 300 } },
    ];
    const wrapper = mount(BaseSegmentedControl, {
      props: {
        name: "lock-mode",
        modelValue: { idle: 300 } as Mode,
        options: opts,
        by: (a: unknown, b: unknown) => (a as Mode).idle === (b as Mode).idle,
      },
    });
    const radios = wrapper.findAll('input[type="radio"]');
    // Default === would never match two distinct objects; only the comparator
    // marks the 5-min preset active.
    expect((radios[0]!.element as HTMLInputElement).checked).toBe(false);
    expect((radios[1]!.element as HTMLInputElement).checked).toBe(true);

    await radios[0]!.trigger("change");
    expect(wrapper.emitted("change")![0]).toEqual([{ idle: 60 }]);
  });

  it("disables the whole group via the `disabled` prop", () => {
    const wrapper = mount(BaseSegmentedControl, {
      props: {
        name: "lock-mode",
        modelValue: 1,
        options: [{ label: "A", value: 1 }],
        disabled: true,
      },
    });
    expect(
      (wrapper.find("fieldset").element as HTMLFieldSetElement).disabled,
    ).toBe(true);
  });
});
