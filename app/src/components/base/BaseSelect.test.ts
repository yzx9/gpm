// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import {
  BACK_HANDLER_KEY,
  createBackHandlerRegistry,
  createScrollLockController,
  SCROLL_LOCK_KEY,
} from "@/composables";
import {
  enableAutoUnmount,
  flushPromises,
  mount,
  type ComponentMountingOptions,
} from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import BaseSelect from "./BaseSelect.vue";

// BaseSelect mounts a BaseModalShell, which locks the document scroller and
// registers an Android back handler. Unmount between tests so the shared
// scroll-lock count returns to 0, and provide both inject keys every mount.
enableAutoUnmount(afterEach);

const backHandler = createBackHandlerRegistry();
function mountSelect(
  options: ComponentMountingOptions<typeof BaseSelect> = {},
) {
  return mount(BaseSelect, {
    ...options,
    global: {
      ...options.global,
      provide: {
        [SCROLL_LOCK_KEY]: createScrollLockController(),
        [BACK_HANDLER_KEY]: backHandler,
      },
    },
  });
}

// Drive "back pressed" — same mock shape as BaseModalShell.test.ts, this file only.
const api = vi.hoisted(() => {
  let handler: ((p: { canGoBack: boolean }) => void) | null = null;
  const unregister = vi.fn(async () => {
    handler = null;
  });
  const onBackButtonPress = vi.fn((h: (p: { canGoBack: boolean }) => void) => {
    handler = h;
    return Promise.resolve({ unregister });
  });
  const fireBack = () => {
    handler?.({ canGoBack: false });
  };
  return { onBackButtonPress, unregister, fireBack };
});
vi.mock("@tauri-apps/api/app", () => ({
  onBackButtonPress: api.onBackButtonPress,
}));

const CADENCE = [
  { label: "Off", value: "off" },
  { label: "1 hour", value: "1h" },
  { label: "6 hours", value: "6h" },
] as const;

async function openSheet(
  wrapper: ReturnType<typeof mountSelect>,
): Promise<void> {
  await wrapper.find("button.trigger").trigger("click");
  await flushPromises();
}

describe("BaseSelect", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders the selected option's label as the trigger text", () => {
    const wrapper = mountSelect({
      props: {
        name: "cadence",
        legend: "Interval",
        modelValue: "6h",
        options: [...CADENCE],
      },
    });
    expect(wrapper.find(".trigger-label").text()).toBe("6 hours");
    // A real selection is not muted.
    expect(wrapper.find(".trigger-label.placeholder").exists()).toBe(false);
  });

  it("shows the placeholder (muted) when no option matches modelValue", () => {
    const wrapper = mountSelect({
      props: {
        name: "cadence",
        legend: "Interval",
        modelValue: "30m", // not in the list
        placeholder: "Pick an interval",
        options: [...CADENCE],
      },
    });
    expect(wrapper.find(".trigger-label").text()).toBe("Pick an interval");
    expect(wrapper.find(".trigger-label.placeholder").exists()).toBe(true);
  });

  it("opens on trigger tap and emits `change` with the picked value, then closes", async () => {
    const wrapper = mountSelect({
      props: {
        name: "cadence",
        legend: "Interval",
        modelValue: "off",
        options: [...CADENCE],
      },
    });
    await openSheet(wrapper);
    const radios = wrapper.findAll('input[type="radio"]');
    expect(radios).toHaveLength(3);

    await radios[2]!.trigger("change"); // pick "6 hours"
    expect(wrapper.emitted("change")![0]).toEqual(["6h"]);
    // Optimistic close: the sheet unmounts right away.
    await flushPromises();
    expect(wrapper.find(".options").exists()).toBe(false);
  });

  it("marks only the matching option's radio checked", async () => {
    const wrapper = mountSelect({
      props: {
        name: "cadence",
        legend: "Interval",
        modelValue: "1h",
        options: [...CADENCE],
      },
    });
    await openSheet(wrapper);
    const radios = wrapper.findAll('input[type="radio"]');
    expect((radios[0]!.element as HTMLInputElement).checked).toBe(false);
    expect((radios[1]!.element as HTMLInputElement).checked).toBe(true);
    expect((radios[2]!.element as HTMLInputElement).checked).toBe(false);
    // The active row carries the accent class + a Check icon.
    expect(wrapper.findAll(".option.active")).toHaveLength(1);
    expect(wrapper.find(".option.active .check").exists()).toBe(true);
  });

  it("closes without emitting `change` when the backdrop is tapped", async () => {
    const wrapper = mountSelect({
      props: {
        name: "cadence",
        legend: "Interval",
        modelValue: "off",
        options: [...CADENCE],
      },
    });
    await openSheet(wrapper);
    await wrapper.find(".overlay").trigger("click");
    await flushPromises();
    expect(wrapper.find(".options").exists()).toBe(false);
    expect(wrapper.emitted("change")).toBeUndefined();
  });

  it("does not open when disabled", async () => {
    const wrapper = mountSelect({
      props: {
        name: "cadence",
        legend: "Interval",
        modelValue: "off",
        options: [...CADENCE],
        disabled: true,
      },
    });
    expect(wrapper.find("button.trigger").attributes("disabled")).toBeDefined();
    await wrapper.find("button.trigger").trigger("click");
    await flushPromises();
    expect(wrapper.find(".options").exists()).toBe(false);
  });

  it("moves focus to the checked radio on open and back to the trigger on close", async () => {
    // jsdom only tracks focus on elements attached to the document, so attach
    // this mount — the others don't assert document.activeElement.
    const wrapper = mountSelect({
      attachTo: document.body,
      props: {
        name: "cadence",
        legend: "Interval",
        modelValue: "1h",
        options: [...CADENCE],
      },
    });
    const button = wrapper.get("button.trigger");
    (button.element as HTMLButtonElement).focus();
    expect(document.activeElement).toBe(button.element);

    await openSheet(wrapper);
    // Focus moved into the sheet — to the checked ("1 hour") radio.
    const radios = wrapper.findAll('input[type="radio"]');
    expect(document.activeElement).toBe(radios[1]!.element);

    await wrapper.find(".overlay").trigger("click"); // backdrop close
    await flushPromises();
    // Focus restored to the trigger, not stranded on <body>.
    expect(document.activeElement).toBe(button.element);
  });

  it("traps Tab inside the sheet — Tab and Shift+Tab cycle among options", async () => {
    const wrapper = mountSelect({
      attachTo: document.body,
      props: {
        name: "cadence",
        legend: "Interval",
        modelValue: "1h",
        options: [...CADENCE],
      },
    });
    await openSheet(wrapper);
    const radios = wrapper.findAll('input[type="radio"]');
    // The checked option ("1 hour", index 1) is focused on open.
    expect(document.activeElement).toBe(radios[1]!.element);
    // Tab → next option (index 2).
    await wrapper.find("fieldset.options").trigger("keydown", { key: "Tab" });
    expect(document.activeElement).toBe(radios[2]!.element);
    // Tab from the last → wraps to the first (index 0).
    await wrapper.find("fieldset.options").trigger("keydown", { key: "Tab" });
    expect(document.activeElement).toBe(radios[0]!.element);
    // Shift+Tab from the first → wraps to the last (index 2).
    await wrapper
      .find("fieldset.options")
      .trigger("keydown", { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(radios[2]!.element);
  });

  it("closes on Android back", async () => {
    const wrapper = mountSelect({
      props: {
        name: "cadence",
        legend: "Interval",
        modelValue: "off",
        options: [...CADENCE],
      },
    });
    await openSheet(wrapper);
    api.fireBack();
    await flushPromises();
    expect(wrapper.find(".options").exists()).toBe(false);
    expect(wrapper.emitted("change")).toBeUndefined();
  });

  it("closes on Escape without emitting change", async () => {
    const wrapper = mountSelect({
      props: {
        name: "cadence",
        legend: "Interval",
        modelValue: "off",
        options: [...CADENCE],
      },
    });
    await openSheet(wrapper);
    await wrapper
      .find("fieldset.options")
      .trigger("keydown", { key: "Escape" });
    await flushPromises();
    expect(wrapper.find(".options").exists()).toBe(false);
    expect(wrapper.emitted("change")).toBeUndefined();
  });

  it("focuses the fieldset and still closes on Escape when there are no options", async () => {
    const wrapper = mountSelect({
      attachTo: document.body,
      props: {
        name: "empty",
        legend: "Empty",
        modelValue: "x",
        options: [],
        emptyLabel: "Nothing here",
      },
    });
    await openSheet(wrapper);
    // No radios — focus falls back to the fieldset so the keyboard contract
    // (ESC closes) still holds.
    expect(document.activeElement).toBe(
      wrapper.find("fieldset.options").element,
    );
    await wrapper
      .find("fieldset.options")
      .trigger("keydown", { key: "Escape" });
    await flushPromises();
    expect(wrapper.find(".options").exists()).toBe(false);
  });
});
