// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> //
// SPDX-License-Identifier: Apache-2.0

import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import BaseOnOffToggle from "./BaseOnOffToggle.vue";

// vue-i18n is installed globally by src/test/setup.ts, so `common.toggle.on/off`
// resolve to "On"/"Off" without per-test wiring.
describe("BaseOnOffToggle", () => {
  it("renders On then Off (On left, Off right) from the shared locale key", () => {
    const wrapper = mount(BaseOnOffToggle, {
      props: { name: "demo", modelValue: true },
    });
    const pills = wrapper.findAll(".mode-pill");
    expect(pills).toHaveLength(2);
    expect(pills[0]!.text()).toBe("On");
    expect(pills[1]!.text()).toBe("Off");
  });

  it("marks the On pill active when true and the Off pill active when false", () => {
    const on = mount(BaseOnOffToggle, {
      props: { name: "demo", modelValue: true },
    });
    const onPills = on.findAll(".mode-pill");
    expect(onPills[0]!.classes()).toContain("mode-active");
    expect(onPills[1]!.classes()).not.toContain("mode-active");

    const off = mount(BaseOnOffToggle, {
      props: { name: "demo", modelValue: false },
    });
    const offPills = off.findAll(".mode-pill");
    expect(offPills[0]!.classes()).not.toContain("mode-active");
    expect(offPills[1]!.classes()).toContain("mode-active");
  });

  it("emits change with the selected boolean", async () => {
    const wrapper = mount(BaseOnOffToggle, {
      props: { name: "demo", modelValue: true },
    });
    // radio[1] is the Off pill (value false).
    await wrapper.findAll('input[type="radio"]')[1]!.trigger("change");
    expect(wrapper.emitted("change")![0]).toEqual([false]);
  });

  it("forwards disabled, legend, and aria-label to the underlying fieldset", () => {
    const wrapper = mount(BaseOnOffToggle, {
      props: {
        name: "demo",
        modelValue: false,
        legend: "Publish on every save",
        ariaLabel: "Background sync",
        disabled: true,
      },
    });
    const fieldset = wrapper.find("fieldset");
    expect((fieldset.element as HTMLFieldSetElement).disabled).toBe(true);
    expect(fieldset.attributes("aria-label")).toBe("Background sync");
    expect(wrapper.find("legend").text()).toBe("Publish on every save");
  });

  it("forwards the #hint slot", () => {
    const wrapper = mount(BaseOnOffToggle, {
      props: { name: "demo", modelValue: true },
      slots: { hint: '<p class="hint">explainer</p>' },
    });
    expect(wrapper.find(".hint").text()).toBe("explainer");
  });

  it("omits the legend and aria-label when not provided", () => {
    const wrapper = mount(BaseOnOffToggle, {
      props: { name: "demo", modelValue: true },
    });
    expect(wrapper.find("legend").exists()).toBe(false);
    expect(wrapper.find("fieldset").attributes("aria-label")).toBeUndefined();
  });
});
