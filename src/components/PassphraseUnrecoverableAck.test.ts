// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import PassphraseUnrecoverableAck from "./PassphraseUnrecoverableAck.vue";

describe("PassphraseUnrecoverableAck", () => {
  it("renders the warning text and an unchecked checkbox by default", () => {
    const wrapper = mount(PassphraseUnrecoverableAck);
    expect(wrapper.text()).toContain("cannot be recovered");
    expect(wrapper.text()).toContain("permanently lock me out");
    const cb = wrapper.find('input[type="checkbox"]');
    expect(cb.exists()).toBe(true);
    expect((cb.element as HTMLInputElement).checked).toBe(false);
  });

  it("checks the box and emits update:modelValue when the user ticks it", async () => {
    const wrapper = mount(PassphraseUnrecoverableAck);
    await wrapper.find('input[type="checkbox"]').setValue(true);
    expect(
      (wrapper.find('input[type="checkbox"]').element as HTMLInputElement)
        .checked,
    ).toBe(true);
    expect(wrapper.emitted("update:modelValue")![0]).toEqual([true]);
  });

  it("reflects a parent-provided modelValue=true (v-model round-trip)", () => {
    const wrapper = mount(PassphraseUnrecoverableAck, {
      props: { modelValue: true },
    });
    expect(
      (wrapper.find('input[type="checkbox"]').element as HTMLInputElement)
        .checked,
    ).toBe(true);
  });

  it("un-checking emits update:modelValue=false", async () => {
    const wrapper = mount(PassphraseUnrecoverableAck, {
      props: { modelValue: true },
    });
    await wrapper.find('input[type="checkbox"]').setValue(false);
    expect(wrapper.emitted("update:modelValue")![0]).toEqual([false]);
  });
});
