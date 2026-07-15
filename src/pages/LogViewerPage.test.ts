// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import { flushPromises } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { mountWithApp } from "@/test/appTestUtils";

import LogViewerPage from "./LogViewerPage.vue";

// Per-file auto-mock of the Tauri core (shadows the global one in setup.ts); the
// test drives `invoke` per-call via `vi.mocked(invoke).mockImplementation`.
vi.mock("@tauri-apps/api/core");

/** The four commands `src/api/log.ts` calls (`read_log`, `get_log_level`,
 *  `set_log_level`, `clear_log`). `routeInvoke` resolves the `ok` map, rejects
 *  the `reject` map, and defaults anything else (set/clear/write) to success. */
function routeInvoke(
  ok: Record<string, unknown>,
  reject: Record<string, unknown> = {},
): void {
  vi.mocked(invoke).mockImplementation((cmd: string) => {
    if (cmd in reject) return Promise.reject(reject[cmd]);
    if (cmd in ok) return Promise.resolve(ok[cmd]);
    return Promise.resolve(undefined);
  });
}

describe("LogViewerPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // confirm() defaults to true (src/test/setup.ts); reset per test.
    vi.mocked(globalThis.confirm).mockReturnValue(true);
    routeInvoke({ read_log: "line one\nline two", get_log_level: "info" });
  });
  afterEach(() => vi.restoreAllMocks());

  it("loads the log text and level on mount", async () => {
    const { wrapper } = mountWithApp(LogViewerPage);
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("read_log");
    expect(invoke).toHaveBeenCalledWith("get_log_level");
    const pre = wrapper.find("pre.log-display");
    expect(pre.exists()).toBe(true);
    expect(pre.text()).toContain("line one");
    expect(pre.text()).toContain("line two");
  });

  it("shows the empty state when the log is empty", async () => {
    routeInvoke({ read_log: "", get_log_level: "info" });
    const { wrapper } = mountWithApp(LogViewerPage);
    await flushPromises();

    expect(wrapper.find("pre.log-display").exists()).toBe(false);
  });

  it("changes the level via the selector (calls set_log_level)", async () => {
    const { wrapper } = mountWithApp(LogViewerPage);
    await flushPromises();

    // LEVELS order: error(0), warn(1), info(2), debug(3).
    const radios = wrapper.findAll('input[type="radio"]');
    expect(radios).toHaveLength(4);
    await radios[3]!.trigger("change"); // → debug
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("set_log_level", { level: "debug" });
  });

  it("clears the log after confirm() (Clear button)", async () => {
    const { wrapper } = mountWithApp(LogViewerPage);
    await flushPromises();

    const clearBtn = wrapper
      .findAll("button")
      .find((b) => b.text().includes("Clear log"));
    expect(clearBtn).toBeTruthy();
    await clearBtn!.trigger("click");
    await flushPromises();

    expect(vi.mocked(globalThis.confirm)).toHaveBeenCalled();
    expect(invoke).toHaveBeenCalledWith("clear_log");
    expect(wrapper.find("pre.log-display").exists()).toBe(false);
  });

  it("aborts clear when confirm() is cancelled", async () => {
    vi.mocked(globalThis.confirm).mockReturnValue(false);
    const { wrapper } = mountWithApp(LogViewerPage);
    await flushPromises();

    const clearBtn = wrapper
      .findAll("button")
      .find((b) => b.text().includes("Clear log"));
    await clearBtn!.trigger("click");
    await flushPromises();

    expect(invoke).not.toHaveBeenCalledWith("clear_log");
    expect(wrapper.find("pre.log-display").text()).toContain("line one");
  });

  it("shows an error alert (not a toast) when read_log fails", async () => {
    routeInvoke({ get_log_level: "info" }, { read_log: { message: "boom" } });
    const { wrapper, toast } = mountWithApp(LogViewerPage);
    const dangerSpy = vi.spyOn(toast.toast, "danger");
    await flushPromises();

    expect(wrapper.findComponent({ name: "BaseAlert" }).exists()).toBe(true);
    expect(dangerSpy).not.toHaveBeenCalled();
  });

  it("toasts danger when set_log_level fails (re-reads the real level)", async () => {
    const { wrapper, toast } = mountWithApp(LogViewerPage);
    const dangerSpy = vi.spyOn(toast.toast, "danger");
    await flushPromises();

    // After load, make set_log_level reject (getLogLevel still resolves, which
    // onLevelChange re-reads on failure).
    routeInvoke(
      { read_log: "x", get_log_level: "info" },
      { set_log_level: { message: "nope" } },
    );
    const radios = wrapper.findAll('input[type="radio"]');
    await radios[0]!.trigger("change"); // → error (fails to persist)
    await flushPromises();

    expect(dangerSpy).toHaveBeenCalled();
    // Re-read reconciled the selector back to the real backend level.
    expect(invoke).toHaveBeenCalledWith("get_log_level");
  });
});
