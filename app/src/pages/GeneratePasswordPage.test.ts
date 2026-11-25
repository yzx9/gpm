// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { DEFAULT_GENERATOR_SETTINGS, type GeneratorSettings } from "@/api";
import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import GeneratePasswordPage from "./GeneratePasswordPage.vue";

const { mockPush, mockReplace } = vi.hoisted(() => ({
  mockPush: vi.fn(),
  mockReplace: vi.fn(),
}));

vi.mock("@tauri-apps/api/core");

vi.mock("vue-router", () => ({
  createRouter: vi.fn(),
  createWebHashHistory: vi.fn(),
  useRouter: () => ({ push: mockPush, replace: mockReplace, back: vi.fn() }),
}));

type Handler = (args?: unknown) => unknown;
type Handlers = Record<string, Handler>;

const DEFAULT_CONFIG = { generator: { ...DEFAULT_GENERATOR_SETTINGS } };

/** Wire `invoke` per command; returns a fresh mounted GeneratePasswordPage.
 *  Auto-generation means every mount needs `get_app_config` + a batch handler. */
async function mountPage(handlers: Handlers = {}) {
  const defaults: Handlers = {
    get_app_config: () => DEFAULT_CONFIG,
    generate_password_batch: () => ["generated"],
    set_generator_settings: () => DEFAULT_CONFIG,
    copy_generated_password: () => undefined,
  };
  vi.mocked(invoke).mockImplementation(((cmd: string, args?: unknown) => {
    const h = handlers[cmd] ?? defaults[cmd];
    if (h) return Promise.resolve(h(args));
    return Promise.resolve(undefined);
  }) as typeof invoke);
  const app = mountWithApp(GeneratePasswordPage);
  await flushPromises();
  return app;
}

function generateCalls() {
  return vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "generate_password_batch");
}

function lastGenerateArgs(): Record<string, unknown> {
  const calls = generateCalls();
  return (calls[calls.length - 1]?.[1] ?? {}) as Record<string, unknown>;
}

function settingsSaves() {
  return vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "set_generator_settings");
}

function lastSavedSettings(): GeneratorSettings {
  const calls = settingsSaves();
  return (calls[calls.length - 1]?.[1] as { settings: GeneratorSettings }).settings;
}

function configWith(generator: Partial<GeneratorSettings>) {
  return { generator: { ...DEFAULT_GENERATOR_SETTINGS, ...generator } };
}

describe("GeneratePasswordPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("auto-generates once on mount with the install defaults", async () => {
    const { wrapper } = await mountPage();

    expect(generateCalls()).toHaveLength(1);
    expect(lastGenerateArgs()).toEqual(
      expect.objectContaining({
        mode: "random",
        count: 1,
        minLen: 24,
        maxLen: 24,
        minDigits: 1,
        minSymbols: 1,
        avoidAmbiguous: true,
      }),
    );
    expect(wrapper.findAll(".current-row")).toHaveLength(1);
    expect(wrapper.find(".current-row").text()).toContain("generated");
  });

  it("restores the persisted recipe before auto-generating", async () => {
    const { wrapper } = await mountPage({
      get_app_config: () =>
        configWith({
          mode: "memorable",
          length: 30,
          count: 2,
          upper: false,
          lower: true,
          digits: false,
          symbols: false,
          minDigits: 0,
          minSymbols: 0,
          avoidAmbiguous: false,
        }),
      generate_password_batch: () => ["first", "second"],
    });

    expect(lastGenerateArgs()).toEqual(
      expect.objectContaining({
        mode: "memorable",
        count: 2,
        minLen: 30,
        maxLen: 30,
        charset: null,
        minDigits: 0,
        minSymbols: 0,
        avoidAmbiguous: false,
      }),
    );
    expect(wrapper.findAll(".current-row")).toHaveLength(2);
    expect((wrapper.find("#g-length").element as HTMLInputElement).value).toBe("30");
    expect((wrapper.find("#g-count").element as HTMLInputElement).value).toBe("2");
  });

  it("falls back to defaults when the config read fails", async () => {
    const { wrapper } = await mountPage({
      get_app_config: () => Promise.reject({ code: "CONFIG_ERROR", message: "boom" }),
    });

    expect(lastGenerateArgs()).toEqual(expect.objectContaining({ mode: "random", count: 1 }));
    expect(wrapper.findAll(".current-row")).toHaveLength(1);
  });

  it("switching the mode buttons regenerates with that generator", async () => {
    const { wrapper } = await mountPage();

    // The three top buttons are the mode fieldset's radios; pick Passphrase.
    await wrapper.findAll('input[type="radio"][name="generate-mode"]')[2]!.trigger("change");
    await flushPromises();

    expect(lastGenerateArgs()).toEqual(
      expect.objectContaining({ mode: "xkcd", minLen: null, maxLen: null, count: 1 }),
    );
    expect(lastSavedSettings()).toEqual(expect.objectContaining({ mode: "xkcd" }));
    expect(wrapper.find("#g-length").exists()).toBe(false);
  });

  it("length and count sliders flow through to the request and persist", async () => {
    const { wrapper } = await mountPage();

    await wrapper.find("#g-length").setValue("32");
    await flushPromises();
    await wrapper.find("#g-count").setValue("3");
    await flushPromises();

    expect(lastGenerateArgs()).toEqual(
      expect.objectContaining({ minLen: 32, maxLen: 32, count: 3 }),
    );
    expect(lastSavedSettings()).toEqual(
      expect.objectContaining({ length: 32, count: 3 }),
    );
  });

  it("sends the selected character groups and advanced quotas", async () => {
    const { wrapper } = await mountPage();

    await wrapper.find("#g-length").setValue("16");
    await flushPromises();
    // Turn off uppercase, lowercase, and digits — only symbols remain.
    const groups = wrapper.findAll('input[type="checkbox"]');
    await groups[0]!.setValue(false);
    await groups[1]!.setValue(false);
    await groups[2]!.setValue(false);
    await flushPromises();
    await wrapper.find("#g-min-symbols").setValue("2");
    await flushPromises();

    expect(lastGenerateArgs()).toEqual(
      expect.objectContaining({
        charset: "!@#$%^&*",
        minDigits: 0,
        minSymbols: 2,
        avoidAmbiguous: true,
        minLen: 16,
        maxLen: 16,
      }),
    );
    expect(lastSavedSettings()).toEqual(
      expect.objectContaining({
        upper: false,
        lower: false,
        digits: false,
        symbols: true,
        minDigits: 0,
        minSymbols: 2,
      }),
    );
  });

  it("disabling a group clears its minimum", async () => {
    const { wrapper } = await mountPage();
    const groups = wrapper.findAll('input[type="checkbox"]');
    expect(groups).toHaveLength(4);
    // Digits is the third group; its minimum resets to 0 when it is disabled.
    await groups[2]!.setValue(false);
    await flushPromises();
    expect((wrapper.find("#g-min-digits").element as HTMLInputElement).disabled).toBe(true);

    expect(lastGenerateArgs()).toEqual(expect.objectContaining({ minDigits: 0 }));
    expect(lastSavedSettings()).toEqual(expect.objectContaining({ minDigits: 0 }));
  });

  it("refuses to disable the last remaining character group", async () => {
    const { wrapper } = await mountPage();
    const groups = wrapper.findAll('input[type="checkbox"]');
    // Leave symbols as the only enabled group.
    await groups[0]!.setValue(false);
    await groups[1]!.setValue(false);
    await groups[2]!.setValue(false);
    await flushPromises();
    expect(lastGenerateArgs()).toEqual(expect.objectContaining({ charset: "!@#$%^&*" }));

    await groups[3]!.setValue(false);
    await flushPromises();

    // The last group cannot be toggled off; the request still has an alphabet.
    expect((wrapper.findAll('input[type="checkbox"]')[3]!.element as HTMLInputElement).checked).toBe(
      true,
    );
    expect(lastGenerateArgs()).toEqual(expect.objectContaining({ charset: "!@#$%^&*" }));
  });

  it("the avoid-ambiguous toggle flows through and persists", async () => {
    const { wrapper } = await mountPage();

    // BaseOnOffToggle: On is the first pill, Off is the second.
    await wrapper.findAll('input[type="radio"][name="generate-avoid-ambiguous"]')[1]!.trigger(
      "change",
    );
    await flushPromises();

    expect(lastGenerateArgs()).toEqual(expect.objectContaining({ avoidAmbiguous: false }));
    expect(lastSavedSettings()).toEqual(expect.objectContaining({ avoidAmbiguous: false }));
  });

  it("keeps the previous batch in history, newest first", async () => {
    let call = 0;
    const { wrapper } = await mountPage({
      generate_password_batch: () => {
        call++;
        return call === 1 ? ["first"] : ["second"];
      },
    });

    expect(wrapper.find(".current-row").text()).toContain("first");

    await wrapper.find('button[aria-label="Regenerate"]').trigger("click");
    await flushPromises();

    expect(wrapper.find(".current-row").text()).toContain("second");
    const history = wrapper.findAll(".history-row");
    expect(history).toHaveLength(1);
    expect(history[0]!.text()).toContain("first");
  });

  it("clears only the earlier history after confirmation", async () => {
    let call = 0;
    const { wrapper, dialog, toast } = await mountPage({
      generate_password_batch: () => {
        call++;
        return call === 1 ? ["current-now"] : ["older"];
      },
    });
    // A first regenerate moves the auto-generated batch into history.
    await wrapper.find('button[aria-label="Regenerate"]').trigger("click");
    await flushPromises();
    // Wait: the mock's second call is the *new* current; the first moved down.
    expect(wrapper.findAll(".history-row")).toHaveLength(1);

    await wrapper.find('button[aria-label="Clear history"]').trigger("click");
    await flushPromises();

    expect(dialog.dialog.confirm).toHaveBeenCalled();
    expect(wrapper.findAll(".history-row")).toHaveLength(0);
    // The current result stays on screen; only earlier batches were cleared.
    expect(wrapper.find(".current-row").text()).toContain("older");
    expect(toast.toasts.value.some((m) => m.message.includes("History cleared"))).toBe(true);
  });

  it("keeps history when the clear confirmation is cancelled", async () => {
    let call = 0;
    const { wrapper, dialog } = await mountPage({
      generate_password_batch: () => {
        call++;
        return call === 1 ? ["first"] : ["second"];
      },
    });
    vi.mocked(dialog.dialog.confirm).mockResolvedValue(false);
    await wrapper.find('button[aria-label="Regenerate"]').trigger("click");
    await flushPromises();

    await wrapper.find('button[aria-label="Clear history"]').trigger("click");
    await flushPromises();

    expect(wrapper.findAll(".history-row")).toHaveLength(1);
  });

  it("copying a row invokes copy_generated_password and toasts", async () => {
    const { wrapper, toast } = await mountPage({
      generate_password_batch: () => ["topsecret"],
    });

    await wrapper.find(".current-row button").trigger("click");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith(
      "copy_generated_password",
      expect.objectContaining({ text: "topsecret" }),
    );
    expect(toast.toasts.value.some((t) => t.message.includes("Copied"))).toBe(true);
  });

  it("a generate error shows the message and renders no rows", async () => {
    const { wrapper } = await mountPage({
      generate_password_batch: () => Promise.reject({ code: "STORE_ERROR", message: "RNG down" }),
    });

    expect(wrapper.text()).toContain("RNG down");
    expect(wrapper.findAll(".current-row")).toHaveLength(0);
  });

  it("clears the current batch and history the moment the identity locks", async () => {
    let call = 0;
    const { wrapper, lock } = await mountPage({
      generate_password_batch: () => {
        call++;
        return call === 1 ? ["a"] : ["b"];
      },
    });
    // Force a history entry, then lock.
    await wrapper.find('button[aria-label="Regenerate"]').trigger("click");
    await flushPromises();
    expect(wrapper.findAll(".current-row")).toHaveLength(1);
    expect(wrapper.findAll(".history-row")).toHaveLength(1);

    // The page registers an onLock clearer via useLockState; drive the same
    // unlocked → locked transition the backend event would (default mount is
    // unlocked, so a single setLocked(true) fires the clearer).
    lock.setLocked(true);
    await flushPromises();

    expect(wrapper.findAll(".current-row")).toHaveLength(0);
    expect(wrapper.findAll(".history-row")).toHaveLength(0);
  });

  it("clears the batch on browser back (popstate)", async () => {
    const { wrapper } = await mountPage({
      generate_password_batch: () => ["a", "b", "c"],
    });
    expect(wrapper.findAll(".current-row")).toHaveLength(3);

    // vue-router is mocked, so drive popstate directly (the real browser-back path).
    window.dispatchEvent(new PopStateEvent("popstate"));
    await flushPromises();

    expect(wrapper.findAll(".current-row")).toHaveLength(0);
  });

  it("drops a stale result that resolves after a lock", async () => {
    let resolveGenerate!: (v: string[]) => void;
    const pending = new Promise<string[]>((r) => {
      resolveGenerate = r;
    });
    const { wrapper, lock } = await mountPage({
      generate_password_batch: () => pending,
    });

    lock.setLocked(true);
    await flushPromises();
    resolveGenerate(["STALE"]);
    await flushPromises();

    expect(wrapper.text()).not.toContain("STALE");
    expect(wrapper.findAll(".current-row")).toHaveLength(0);
  });

  it("Back returns to the entry list", async () => {
    const { wrapper } = await mountPage();
    await wrapper.find('button[aria-label="Back"]').trigger("click");
    expect(mockReplace).toHaveBeenCalledWith({ name: "entries" });
  });

  // ── Gate re-lock (issue #20): the mask does not unmount the page ────────

  it("a gate re-lock clears the auto-generated batch and marks the drafts notice", async () => {
    const m = await mountPage({
      generate_password_batch: () => ["a", "b", "c"],
    });
    expect(m.wrapper.findAll(".current-row")).toHaveLength(3);

    m.appLock.setAppLocked(true, "idle");
    await flushPromises();

    expect(m.wrapper.findAll(".current-row")).toHaveLength(0);
    expect(m.draftsNotice.consume()).toBe(true); // batch lost → toast fires
  });

  it("a gate re-lock with no generated batch does not mark the notice", async () => {
    const m = await mountPage({
      generate_password_batch: () => Promise.reject({ code: "STORE_ERROR", message: "RNG down" }),
    });

    m.appLock.setAppLocked(true, "idle");
    await flushPromises();

    expect(m.draftsNotice.consume()).toBe(false); // nothing was lost
  });
});
