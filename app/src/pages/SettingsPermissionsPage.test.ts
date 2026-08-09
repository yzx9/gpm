// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import BaseSpinner from "@/components/base/BaseSpinner.vue";
import { mountWithApp } from "@/test/appTestUtils";
import {
  baseDefaults,
  resetOverrides,
  type Overrides,
} from "@/test/settingsTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  flushPromises,
  type DOMWrapper,
  type VueWrapper,
} from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import SettingsPermissionsPage from "./SettingsPermissionsPage.vue";

const { mockPush } = vi.hoisted(() => ({ mockPush: vi.fn() }));

vi.mock("vue-router", () => ({
  createRouter: vi.fn(),
  createWebHashHistory: vi.fn(),
  useRouter: () => ({ push: mockPush, replace: vi.fn(), back: vi.fn() }),
  useRoute: () => ({
    params: {},
    query: {},
    name: "",
    path: "/",
    fullPath: "/",
  }),
}));

/** Resolve the `subscribeAppResume` handler captured on the mocked `listen` (the
 *  authoritative `app-resumed` signal, R029) and fire it, simulating an Android
 *  `Activity.onResume`. */
function fireResume() {
  const call = vi.mocked(listen).mock.calls.find((c) => c[0] === "app-resumed");
  // Fail loudly if the resume listener never registered — without this the
  // negative tests below pass vacuously (no handler to fire).
  expect(call).toBeDefined();
  (call?.[1] as () => void)?.();
}

describe("SettingsPermissionsPage", () => {
  const overrides: Overrides = {};
  const defaults = { ...baseDefaults };

  function when(cmd: string, value: unknown) {
    overrides[cmd] = { value };
  }
  function installMock() {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd in overrides) {
        const o = overrides[cmd];
        if (o && o.reject !== undefined) return Promise.reject(o.reject);
        return Promise.resolve(o ? o.value : defaults[cmd]);
      }
      return Promise.resolve(defaults[cmd]);
    });
  }

  beforeEach(() => {
    vi.clearAllMocks();
    resetOverrides(overrides);
    installMock();
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  function mountPage(secureAvailable = true) {
    return mountWithApp(SettingsPermissionsPage, { secureAvailable });
  }

  function rowByText(wrapper: VueWrapper, text: string): DOMWrapper<Element> {
    const el = wrapper
      .findAll(".perm-row")
      .find((r) => r.text().includes(text));
    return el as unknown as DOMWrapper<Element>;
  }

  it("renders title, both group labels, and all 5 rows on Android", async () => {
    when("are_clipboard_notifications_enabled", true);
    when("is_biometric_available", "available");
    const { wrapper } = mountPage();
    await flushPromises();
    expect(wrapper.text()).toContain("Permissions & data");
    expect(wrapper.text()).toContain("Permissions you can change");
    expect(wrapper.text()).toContain("Data access notes");
    expect(wrapper.findAll(".perm-row")).toHaveLength(5);
  });

  it("on desktop hides the adjustable group, shows only the 3 info rows", async () => {
    const { wrapper } = mountPage(false);
    await flushPromises();
    expect(wrapper.text()).not.toContain("Notifications");
    expect(wrapper.findAll(".perm-row")).toHaveLength(3);
    // The clipboard/network/files explainers are still present.
    expect(wrapper.text()).toContain("Clipboard");
    expect(wrapper.text()).toContain("Network");
  });

  it("notification blocked → whole row tappable; click opens settings; opened=false toasts", async () => {
    when("are_clipboard_notifications_enabled", false);
    when("is_biometric_available", "available");
    when("open_clipboard_notification_settings", false);
    const { wrapper, toast } = mountPage();
    await flushPromises();
    const row = rowByText(wrapper, "Notifications");
    expect(row.attributes("role")).toBe("button");
    expect(row.attributes("tabindex")).toBe("0");
    expect(row.text()).toContain("Off");
    await row.trigger("click");
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith("open_clipboard_notification_settings");
    expect(toast.toasts.value.some((t) => t.variant === "danger")).toBe(true);
  });

  it("notification granted → not tappable, shows Enabled", async () => {
    when("are_clipboard_notifications_enabled", true);
    when("is_biometric_available", "available");
    const { wrapper } = mountPage();
    await flushPromises();
    const row = rowByText(wrapper, "Notifications");
    expect(row.attributes("role")).toBeUndefined();
    expect(row.text()).toContain("Enabled");
  });

  it("biometric no_enrollment → tappable; click opens security settings", async () => {
    when("are_clipboard_notifications_enabled", true);
    when("is_biometric_available", "no_enrollment");
    when("open_security_settings", true);
    const { wrapper } = mountPage();
    await flushPromises();
    const row = rowByText(wrapper, "Biometric unlock");
    expect(row.attributes("role")).toBe("button");
    await row.trigger("click");
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith("open_security_settings");
  });

  it("biometric unavailable → not tappable, shows the unavailable status", async () => {
    when("are_clipboard_notifications_enabled", true);
    when("is_biometric_available", "unavailable");
    const { wrapper } = mountPage();
    await flushPromises();
    const row = rowByText(wrapper, "Biometric unlock");
    expect(row.attributes("role")).toBeUndefined();
    expect(row.text()).toContain("Not available on this device");
  });

  it("biometric weak_enrolled → tappable (Class-3 enrollment can help)", async () => {
    when("are_clipboard_notifications_enabled", true);
    when("is_biometric_available", "weak_enrolled");
    when("open_security_settings", true);
    const { wrapper } = mountPage();
    await flushPromises();
    const row = rowByText(wrapper, "Biometric unlock");
    expect(row.attributes("role")).toBe("button");
    await row.trigger("click");
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith("open_security_settings");
  });

  it("notification probe failure → degrades to tappable recovery (no infinite spinner)", async () => {
    overrides["are_clipboard_notifications_enabled"] = {
      reject: new Error("plugin gone"),
    };
    when("is_biometric_available", "available");
    const { wrapper } = mountPage();
    await flushPromises();
    const row = rowByText(wrapper, "Notifications");
    // A flaky probe must not strand the row spinning forever — degrade to the
    // blocked affordance so the recovery deep-link stays reachable.
    expect(row.attributes("role")).toBe("button");
    expect(row.findComponent(BaseSpinner).exists()).toBe(false);
  });

  it("app-resumed re-runs the probe (return-from-settings refresh)", async () => {
    when("are_clipboard_notifications_enabled", false);
    when("is_biometric_available", "available");
    const { wrapper } = mountPage();
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith("are_clipboard_notifications_enabled");
    vi.mocked(invoke).mockClear();
    // The user returns from the system settings screen.
    fireResume();
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith("are_clipboard_notifications_enabled");
    wrapper.unmount(); // also exercises the resume-unlisten teardown path
  });

  it("biometric no_enrollment + open_security_settings opened=false → danger toast", async () => {
    when("are_clipboard_notifications_enabled", true);
    when("is_biometric_available", "no_enrollment");
    when("open_security_settings", false);
    const { wrapper, toast } = mountPage();
    await flushPromises();
    await rowByText(wrapper, "Biometric unlock").trigger("click");
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith("open_security_settings");
    expect(toast.toasts.value.some((t) => t.variant === "danger")).toBe(true);
  });

  it("biometric deep-link throw is caught and toasted", async () => {
    when("are_clipboard_notifications_enabled", true);
    when("is_biometric_available", "no_enrollment");
    overrides["open_security_settings"] = {
      reject: new Error("ActivityNotFound"),
    };
    const { wrapper, toast } = mountPage();
    await flushPromises();
    await rowByText(wrapper, "Biometric unlock").trigger("click");
    await flushPromises();
    expect(toast.toasts.value.some((t) => t.variant === "danger")).toBe(true);
  });

  it("notification deep-link throw is caught and toasted", async () => {
    when("are_clipboard_notifications_enabled", false);
    when("is_biometric_available", "available");
    overrides["open_clipboard_notification_settings"] = {
      reject: new Error("ActivityNotFound"),
    };
    const { wrapper, toast } = mountPage();
    await flushPromises();
    await rowByText(wrapper, "Notifications").trigger("click");
    await flushPromises();
    expect(toast.toasts.value.some((t) => t.variant === "danger")).toBe(true);
  });

  it("discards a stale probe result when a newer probe resolves first", async () => {
    // The generation guard (probeGen) is the whole point of the race defense;
    // a slower earlier probe must not overwrite the fresher state.
    let resolveFirst!: (v: boolean) => void;
    const firstPromise = new Promise<boolean>((r) => (resolveFirst = r));
    let notifCalls = 0;
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "are_clipboard_notifications_enabled") {
        notifCalls++;
        // First probe (onMounted) stays pending; the resume probe resolves fast.
        return notifCalls === 1 ? firstPromise : Promise.resolve(true);
      }
      return Promise.resolve("available");
    });
    const { wrapper } = mountPage();
    await flushPromises();
    // Trigger a newer probe before the first resolves.
    fireResume();
    await flushPromises();
    // Now resolve the STALE first probe with the opposite value.
    resolveFirst(false);
    await flushPromises();
    const row = rowByText(wrapper, "Notifications");
    // Stale 'false' is discarded; the row shows the fresh 'true' (granted, not tappable).
    expect(row.attributes("role")).toBeUndefined();
    expect(row.text()).toContain("Enabled");
  });

  it("unmount unregisters the resume listener (no leak)", async () => {
    when("are_clipboard_notifications_enabled", false);
    when("is_biometric_available", "available");
    const { wrapper } = mountPage();
    await flushPromises();
    // The resume listener is the subscribeAppResume handle; capture its unlisten
    // (the value `listen` resolved to) and assert unmount invokes it.
    const resumeIdx = vi
      .mocked(listen)
      .mock.calls.findIndex((c) => c[0] === "app-resumed");
    const unlisten = await vi.mocked(listen).mock.results[resumeIdx].value;
    expect(unlisten).not.toHaveBeenCalled();
    wrapper.unmount();
    // The handle stored on mount is invoked on unmount — no leaked listener.
    expect(unlisten).toHaveBeenCalled();
  });

  it("releases the resume handle if the page unmounts during the IPC round-trip", async () => {
    when("are_clipboard_notifications_enabled", false);
    when("is_biometric_available", "available");
    const { wrapper } = mountPage();
    // Unmount BEFORE flushing the subscribeAppResume round-trip — the disposed
    // guard must release the late-resolving handle instead of leaking it on a
    // stale (already-unmounted) closure.
    wrapper.unmount();
    await flushPromises();
    const resumeIdx = vi
      .mocked(listen)
      .mock.calls.findIndex((c) => c[0] === "app-resumed");
    const unlisten = await vi.mocked(listen).mock.results[resumeIdx].value;
    expect(unlisten).toHaveBeenCalled();
  });

  it("biometric available + enabled → Enabled status, Manage link to the biometric card", async () => {
    when("are_clipboard_notifications_enabled", true);
    when("is_biometric_available", "available");
    when("is_biometric_unlock_enabled", true);
    when("get_auth_state", {
      configured: true,
      encrypted: true,
      unlocked: false,
      identity_type: "x25519",
    });
    const { wrapper } = mountPage();
    await flushPromises();
    expect(rowByText(wrapper, "Biometric unlock").text()).toContain("Enabled");
    const link = wrapper.find(".perm-link");
    expect(link.exists()).toBe(true);
    expect(link.text()).toContain("Manage");
    await link.trigger("click");
    expect(mockPush).toHaveBeenCalledWith({
      name: "settingsIdentity",
      query: { focus: "biometric" },
    });
  });

  it("biometric available + not enabled → Ready status, Turn-on link to the biometric card", async () => {
    when("are_clipboard_notifications_enabled", true);
    when("is_biometric_available", "available");
    when("get_auth_state", {
      configured: true,
      encrypted: true,
      unlocked: false,
      identity_type: "x25519",
    });
    const { wrapper } = mountPage();
    await flushPromises();
    expect(rowByText(wrapper, "Biometric unlock").text()).toContain("Ready");
    const link = wrapper.find(".perm-link");
    expect(link.exists()).toBe(true);
    expect(link.text()).toContain("Turn on");
    await link.trigger("click");
    expect(mockPush).toHaveBeenCalledWith({
      name: "settingsIdentity",
      query: { focus: "biometric" },
    });
  });

  it("biometric available but identity unencrypted → link points at the passphrase card", async () => {
    when("are_clipboard_notifications_enabled", true);
    when("is_biometric_available", "available");
    // get_auth_state defaults to encrypted=false; is_biometric_unlock_enabled=false
    const { wrapper } = mountPage();
    await flushPromises();
    const link = wrapper.find(".perm-link");
    expect(link.exists()).toBe(true);
    await link.trigger("click");
    expect(mockPush).toHaveBeenCalledWith({
      name: "settingsIdentity",
      query: { focus: "passphrase" },
    });
  });

  it("biometric unavailable → no manage link (nothing to configure there)", async () => {
    when("are_clipboard_notifications_enabled", true);
    when("is_biometric_available", "unavailable");
    const { wrapper } = mountPage();
    await flushPromises();
    expect(wrapper.find(".perm-link").exists()).toBe(false);
  });

  it("biometric available + SSH identity → no manage link (biometric can't apply)", async () => {
    when("are_clipboard_notifications_enabled", true);
    when("is_biometric_available", "available");
    when("get_auth_state", {
      configured: true,
      encrypted: false,
      unlocked: false,
      identity_type: "ssh_ed25519",
    });
    const { wrapper } = mountPage();
    await flushPromises();
    expect(wrapper.find(".perm-link").exists()).toBe(false);
  });
});
