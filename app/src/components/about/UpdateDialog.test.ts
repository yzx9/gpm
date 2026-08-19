// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import type { UpdateStatus } from "@/api";
import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import UpdateDialog from "./UpdateDialog.vue";

vi.mock("@tauri-apps/api/core");
vi.mock("@/utils/open-external", () => ({
  openExternal: vi.fn().mockResolvedValue(undefined),
}));

const RELEASES_URL = "https://github.com/yzx9/gpm/releases/latest";
// Resolve after import so the module mock above is in place.
const { openExternal } = await import("@/utils/open-external");

// Per-command overrides; `check_update_now` and `set_update_check` are driven
// per test.
const overrides: Record<string, unknown> = {};

const AVAILABLE: UpdateStatus = {
  available: true,
  unacknowledged: true,
  latest_version: "v0.19.0",
};
// A completed probe that found nothing newer — a non-null `latest_version` is
// what distinguishes this from "never probed" below.
const UP_TO_DATE: UpdateStatus = {
  available: false,
  unacknowledged: false,
  latest_version: "v0.19.0",
};
// The backend's quiet view: check on, but no probe has ever succeeded (cache
// still loading, or every probe failed). Must NOT render as "up to date".
const NEVER_PROBED: UpdateStatus = {
  available: false,
  unacknowledged: false,
  latest_version: null,
};

function installMock() {
  vi.mocked(invoke).mockImplementation((cmd: string) => {
    if (cmd in overrides) return Promise.resolve(overrides[cmd]);
    return Promise.resolve(undefined);
  });
}

function mountDialog(props: { enabled: boolean; status: UpdateStatus | null }) {
  return mountWithApp(UpdateDialog, { mountOpts: { props } }).wrapper;
}

/** The stacked action buttons: [0] primary, [1] explicit cancel. */
function actions(wrapper: VueWrapper) {
  const buttons = wrapper.findAll(".dialog-actions button");
  return { primary: buttons[0]!, cancel: buttons[1]! };
}

describe("UpdateDialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    for (const k of Object.keys(overrides)) delete overrides[k];
    installMock();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("shows the download action when an update is known (check on)", async () => {
    const wrapper = mountDialog({ enabled: true, status: AVAILABLE });

    expect(wrapper.text()).toContain("Update available");
    expect(wrapper.text()).toContain("v0.19.0");
    expect(actions(wrapper).primary.text()).toBe("Go to download page");

    await actions(wrapper).primary.trigger("click");
    await flushPromises();

    expect(openExternal).toHaveBeenCalledWith(RELEASES_URL);
    expect(wrapper.emitted("close")).toHaveLength(1);
  });

  it("shows up-to-date with Got it when the cache says nothing newer", async () => {
    const wrapper = mountDialog({ enabled: true, status: UP_TO_DATE });

    expect(wrapper.text()).toContain("Up to date");
    expect(actions(wrapper).primary.text()).toBe("Got it");

    await actions(wrapper).primary.trigger("click");
    expect(wrapper.emitted("close")).toHaveLength(1);
    // No probe runs from this view — the check-on path trusts the ≤1/day cache.
    expect(invoke).not.toHaveBeenCalledWith("check_update_now");
  });

  it("offers a manual check when auto-check is off, and shows its result", async () => {
    overrides.check_update_now = AVAILABLE;
    const wrapper = mountDialog({ enabled: false, status: UP_TO_DATE });

    expect(wrapper.text()).toContain("Automatic checks are off");
    expect(actions(wrapper).primary.text()).toBe("Check for updates");

    await actions(wrapper).primary.trigger("click");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("check_update_now");
    // The result replaces the off view — still one interface; the enable
    // affordance stays demoted to the footer link.
    expect(wrapper.text()).toContain("Update available");
    expect(actions(wrapper).primary.text()).toBe("Go to download page");
  });

  it("fails loud: a rejected manual check shows Retry instead of up-to-date", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "check_update_now")
        return Promise.reject(new Error("offline"));
      return Promise.resolve(undefined);
    });
    const wrapper = mountDialog({ enabled: false, status: UP_TO_DATE });

    await actions(wrapper).primary.trigger("click");
    await flushPromises();

    expect(wrapper.text()).toContain("Couldn't check for updates");
    expect(actions(wrapper).primary.text()).toBe("Retry");

    // Retry goes through the same command.
    await actions(wrapper).primary.trigger("click");
    await flushPromises();
    const probeCalls = vi
      .mocked(invoke)
      .mock.calls.filter(
        (call: [string, ...unknown[]]) => call[0] === "check_update_now",
      );
    expect(probeCalls).toHaveLength(2);
    expect(wrapper.text()).toContain("Couldn't check for updates");
  });

  it("cancel emits close without any IPC", async () => {
    const wrapper = mountDialog({ enabled: true, status: UP_TO_DATE });

    await actions(wrapper).cancel.trigger("click");

    expect(wrapper.emitted("close")).toHaveLength(1);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("the footer link toggles the pref, flips in place, and emits the config", async () => {
    overrides.set_update_check = { update_check_enabled: false };
    const wrapper = mountDialog({ enabled: true, status: UP_TO_DATE });
    const link = () => wrapper.find(".pref-link");

    expect(link().text()).toBe("Turn off automatic checks");

    await link().trigger("click");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("set_update_check", { enabled: false });
    expect(wrapper.emitted("pref-changed")![0]).toEqual([
      { update_check_enabled: false },
    ]);
    // Dialog stays open and the view flips to the off state.
    expect(wrapper.emitted("close")).toBeUndefined();
    expect(wrapper.text()).toContain("Automatic checks are off");
    expect(link().text()).toBe("Turn on automatic checks");
  });

  it("enabling via the link also runs a check instead of claiming up-to-date", async () => {
    overrides.set_update_check = { update_check_enabled: true };
    overrides.check_update_now = UP_TO_DATE;
    const wrapper = mountDialog({ enabled: false, status: UP_TO_DATE });

    await wrapper.find(".pref-link").trigger("click");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("set_update_check", { enabled: true });
    expect(invoke).toHaveBeenCalledWith("check_update_now");
    expect(wrapper.text()).toContain("Up to date");
  });

  it("a failed toggle leaves the view on the current pref", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "set_update_check")
        return Promise.reject(new Error("sealed"));
      return Promise.resolve(undefined);
    });
    const wrapper = mountDialog({ enabled: true, status: UP_TO_DATE });

    await wrapper.find(".pref-link").trigger("click");
    await flushPromises();

    expect(wrapper.emitted("pref-changed")).toBeUndefined();
    // Still the check-on view; the link still offers to disable.
    expect(wrapper.text()).toContain("Up to date");
    expect(wrapper.find(".pref-link").text()).toBe("Turn off automatic checks");
  });

  it("a failed toggle surfaces a danger toast (the only feedback on that path)", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "set_update_check")
        return Promise.reject(new Error("sealed"));
      return Promise.resolve(undefined);
    });
    const mounted = mountWithApp(UpdateDialog, {
      mountOpts: { props: { enabled: true, status: UP_TO_DATE } },
    });

    await mounted.wrapper.find(".pref-link").trigger("click");
    await flushPromises();

    expect(mounted.toast.toasts.value).toHaveLength(1);
    expect(mounted.toast.toasts.value[0]!.variant).toBe("danger");
  });

  it("probes on open when the check is on but no probe has ever succeeded", async () => {
    // Covers both the null-status window (config still loading) and a failed
    // cold-start probe: neither may render as "up to date".
    overrides.check_update_now = UP_TO_DATE;
    const wrapper = mountDialog({ enabled: true, status: NEVER_PROBED });
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("check_update_now");
    expect(wrapper.text()).toContain("Up to date");
    expect(wrapper.emitted("checked")).toHaveLength(1);

    // The same holds for a null status (nothing loaded at all).
    vi.mocked(invoke).mockClear();
    overrides.check_update_now = AVAILABLE;
    const unloaded = mountDialog({ enabled: true, status: null });
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("check_update_now");
    expect(unloaded.text()).toContain("Update available");
  });

  it("cancel stays live while checking; a late result after close is harmless", async () => {
    let resolveProbe!: (v: unknown) => void;
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "check_update_now")
        return new Promise((r) => {
          resolveProbe = r;
        });
      return Promise.resolve(undefined);
    });
    const wrapper = mountDialog({ enabled: false, status: UP_TO_DATE });

    await actions(wrapper).primary.trigger("click");
    await flushPromises();

    // Checking state: primary is loading; cancel + the footer link are the
    // labeled escapes (cancel by design, the link disabled to avoid a
    // toggle racing the probe).
    expect(wrapper.text()).toContain("Checking for updates");
    expect(actions(wrapper).cancel.attributes("disabled")).toBeUndefined();
    expect(wrapper.find(".pref-link").attributes("disabled")).toBeDefined();

    // Cancel closes the dialog even though the probe is still in flight…
    await actions(wrapper).cancel.trigger("click");
    expect(wrapper.emitted("close")).toHaveLength(1);

    // …and the probe resolving onto the closed dialog must not throw.
    resolveProbe(AVAILABLE);
    await flushPromises();
  });

  it("re-enabling with a fresh result on screen does not re-probe it away", async () => {
    // Manual check found an update; enabling auto-check must keep showing it
    // instead of racing a second probe that could transiently fail.
    overrides.check_update_now = AVAILABLE;
    overrides.set_update_check = { update_check_enabled: true };
    const wrapper = mountDialog({ enabled: false, status: UP_TO_DATE });

    await actions(wrapper).primary.trigger("click");
    await flushPromises();
    expect(wrapper.text()).toContain("Update available");

    await wrapper.find(".pref-link").trigger("click");
    await flushPromises();

    const probes = vi
      .mocked(invoke)
      .mock.calls.filter(
        (call: [string, ...unknown[]]) => call[0] === "check_update_now",
      );
    expect(probes).toHaveLength(1);
    // The known result survives the toggle.
    expect(wrapper.text()).toContain("Update available");
    expect(wrapper.text()).not.toContain("Couldn't check for updates");
  });

  it("syncs to the pref prop when the parent's config lands late", async () => {
    // The dialog can open before the parent's getAppConfig resolves (it passes
    // the default-on mirror); the real pref must win once it arrives.
    const wrapper = mountDialog({ enabled: true, status: UP_TO_DATE });

    await wrapper.setProps({ enabled: false });
    await flushPromises();

    expect(wrapper.text()).toContain("Automatic checks are off");
    expect(wrapper.find(".pref-link").text()).toBe("Turn on automatic checks");
  });
});
