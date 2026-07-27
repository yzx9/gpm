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

/** The commands this page calls: `read_log` + `get_app_config` on load,
 *  `clear_log` on clear, `set_verbose` on toggle. `routeInvoke` resolves the
 *  `ok` map, rejects the `reject` map, and defaults anything else to success. */
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
    // Default: log text present, no verbose deadline ⇒ toggle Off.
    routeInvoke({ read_log: "line one\nline two", get_app_config: {} });
  });
  afterEach(() => vi.restoreAllMocks());

  it("loads the log text on mount", async () => {
    const { wrapper } = mountWithApp(LogViewerPage);
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("read_log");
    const pre = wrapper.find("pre.log-display");
    expect(pre.exists()).toBe(true);
    expect(pre.text()).toContain("line one");
    expect(pre.text()).toContain("line two");
  });

  it("renders a verbose On/Off toggle, Off by default", async () => {
    const { wrapper } = mountWithApp(LogViewerPage);
    await flushPromises();

    expect(
      wrapper.findComponent({ name: "BaseSegmentedControl" }).exists(),
    ).toBe(true);
    const radios = wrapper.findAll('input[name="verbose"]');
    expect(radios).toHaveLength(2);
    // No verbose_until ⇒ the Off option (second) is checked.
    expect((radios[1].element as HTMLInputElement).checked).toBe(true);
  });

  it("turning verbose on calls set_verbose and notifies", async () => {
    routeInvoke({
      read_log: "line one\nline two",
      get_app_config: {},
      // set_verbose returns the post-enable config (a fresh deadline) so the
      // success path is exercised — otherwise appConfig stays undefined and the
      // state-gated toast/countdown never fire.
      set_verbose: { verbose_until: Math.floor(Date.now() / 1000) + 600 },
    });
    const { wrapper, toast } = mountWithApp(LogViewerPage);
    const infoSpy = vi.spyOn(toast.toast, "info");
    await flushPromises();

    const radios = wrapper.findAll('input[name="verbose"]');
    await radios[0].trigger("change"); // On
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith(
      "set_verbose",
      expect.objectContaining({
        enabled: true,
        revertNotify: expect.objectContaining({
          title: expect.any(String),
          body: expect.any(String),
        }),
      }),
    );
    expect(infoSpy).toHaveBeenCalled();
    // Stop the live countdown the enable path started (avoids a leaked timer).
    wrapper.unmount();
  });

  it("turning verbose off calls set_verbose(false) and does not notify", async () => {
    routeInvoke({
      read_log: "",
      get_app_config: { verbose_until: Math.floor(Date.now() / 1000) + 300 },
      set_verbose: {},
    });
    const { wrapper, toast } = mountWithApp(LogViewerPage);
    const infoSpy = vi.spyOn(toast.toast, "info");
    await flushPromises();

    const radios = wrapper.findAll('input[name="verbose"]');
    await radios[1].trigger("change"); // Off
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith(
      "set_verbose",
      expect.objectContaining({ enabled: false }),
    );
    expect(infoSpy).not.toHaveBeenCalled(); // the Off path does not toast
    wrapper.unmount();
  });

  it("renders the off hint when verbose is not configured", async () => {
    const { wrapper } = mountWithApp(LogViewerPage);
    await flushPromises();
    expect(wrapper.text()).toContain("Turn on to capture everything");
  });

  it("renders the on hint with a countdown while verbose is active", async () => {
    routeInvoke({
      read_log: "",
      get_app_config: { verbose_until: Math.floor(Date.now() / 1000) + 595 },
    });
    const { wrapper } = mountWithApp(LogViewerPage);
    await flushPromises();
    expect(wrapper.text()).toContain("Debug logging is on");
    expect(wrapper.text()).toMatch(/\d{1,2}:\d\d/); // a m:ss countdown renders
    wrapper.unmount(); // stop the live countdown started on load
  });

  it("renders the elapsed hint when the deadline has passed", async () => {
    routeInvoke({
      read_log: "",
      get_app_config: { verbose_until: Math.floor(Date.now() / 1000) - 60 },
    });
    const { wrapper } = mountWithApp(LogViewerPage);
    await flushPromises();
    expect(wrapper.text()).toContain("verbose window has elapsed");
  });

  it("shows verbose On when a deadline is set, even if elapsed", async () => {
    // An expired deadline is still "On" this session (no mid-session revert —
    // the level stays Debug until the next launch clears it; RFC 0055).
    routeInvoke({
      read_log: "",
      get_app_config: { verbose_until: Math.floor(Date.now() / 1000) - 60 },
    });
    const { wrapper } = mountWithApp(LogViewerPage);
    await flushPromises();

    const radios = wrapper.findAll('input[name="verbose"]');
    expect((radios[0].element as HTMLInputElement).checked).toBe(true);
  });

  it("shows the empty state when the log is empty", async () => {
    routeInvoke({ read_log: "" });
    const { wrapper } = mountWithApp(LogViewerPage);
    await flushPromises();

    expect(wrapper.find("pre.log-display").exists()).toBe(false);
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
    routeInvoke({}, { read_log: { message: "boom" } });
    const { wrapper, toast } = mountWithApp(LogViewerPage);
    const dangerSpy = vi.spyOn(toast.toast, "danger");
    await flushPromises();

    expect(wrapper.findComponent({ name: "BaseAlert" }).exists()).toBe(true);
    expect(dangerSpy).not.toHaveBeenCalled();
  });
});
