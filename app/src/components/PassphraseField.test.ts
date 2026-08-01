// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import PassphraseField from "./PassphraseField.vue";

// Stable ids make the main/confirm inputs addressable without relying on
// `useId` output or DOM ordering.
const PROPS = { id: "pf-main" } as const;
const main = (w: ReturnType<typeof mount>) => w.get('input[id="pf-main"]');
const confirm = (w: ReturnType<typeof mount>) =>
  w.find('input[id="pf-main-confirm"]');

describe("PassphraseField", () => {
  it("optional + empty: valid and renders no confirm field", () => {
    const w = mount(PassphraseField, {
      props: { ...PROPS, modelValue: "", optional: true },
    });
    expect(w.vm.validate()).toBeNull();
    expect(confirm(w).exists()).toBe(false);
  });

  it("required + empty: reports the field as required", () => {
    const w = mount(PassphraseField, {
      props: { ...PROPS, modelValue: "", label: "New passphrase" },
    });
    expect(w.vm.validate()).toBe("New passphrase is required");
  });

  it("optional + typed main without confirm: prompts to confirm", () => {
    const w = mount(PassphraseField, {
      props: { ...PROPS, modelValue: "secret", optional: true },
    });
    expect(confirm(w).exists()).toBe(true);
    expect(w.vm.validate()).toBe("Please confirm your passphrase");
  });

  it("matching confirm: valid, no inline mismatch hint", async () => {
    const w = mount(PassphraseField, {
      props: { ...PROPS, modelValue: "secret", optional: true },
    });
    await confirm(w).setValue("secret");
    expect(w.vm.validate()).toBeNull();
    expect(w.text()).not.toContain("Passphrases do not match");
  });

  it("mismatched confirm: returns the mismatch error and shows the hint", async () => {
    const w = mount(PassphraseField, {
      props: { ...PROPS, modelValue: "secret", optional: true },
    });
    await confirm(w).setValue("different");
    expect(w.vm.validate()).toBe("Passphrases do not match");
    expect(w.text()).toContain("Passphrases do not match");
  });

  it("the eye toggle reveals and hides the passphrase", async () => {
    const w = mount(PassphraseField, {
      props: { ...PROPS, modelValue: "secret", optional: true },
    });
    expect(main(w).attributes("type")).toBe("password");
    await w.find('button[aria-label="Show passphrase"]').trigger("click");
    expect(main(w).attributes("type")).toBe("text");
    // One toggle controls both fields.
    expect(confirm(w).attributes("type")).toBe("text");
    await w.find('button[aria-label="Hide passphrase"]').trigger("click");
    expect(main(w).attributes("type")).toBe("password");
  });

  it("reset() clears the confirm field so a re-check flags it empty", async () => {
    const w = mount(PassphraseField, {
      props: { ...PROPS, modelValue: "secret", optional: true },
    });
    await confirm(w).setValue("secret");
    expect(w.vm.validate()).toBeNull();
    w.vm.reset();
    expect(w.vm.validate()).toBe("Please confirm your passphrase");
  });

  it("clearing the main field clears a stale confirm (optional flow)", async () => {
    const w = mount(PassphraseField, {
      props: { ...PROPS, modelValue: "secret", optional: true },
    });
    await confirm(w).setValue("secret");
    expect(w.vm.validate()).toBeNull();
    // Parent clears the main field — the confirm must not linger as hidden state.
    await w.setProps({ modelValue: "" });
    expect(w.vm.validate()).toBeNull();
    expect(confirm(w).exists()).toBe(false);
  });

  it("renders the help slot", () => {
    const w = mount(PassphraseField, {
      props: { ...PROPS, modelValue: "" },
      slots: { help: "<p class='hint'>encrypt at rest</p>" },
    });
    expect(w.text()).toContain("encrypt at rest");
  });
});
