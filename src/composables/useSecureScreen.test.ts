// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createSecureScreen } from "./useSecureScreen";

vi.mock("@tauri-apps/api/core");

const fn = () => invoke as ReturnType<typeof vi.fn>;

describe("useSecureScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("applySecureForRoute is a no-op (returns true, no invoke) when the plugin is unavailable (desktop)", async () => {
    const { applySecureForRoute } = createSecureScreen();
    const ok = await applySecureForRoute(true);
    expect(ok).toBe(true);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("applySecureForRoute sets secure=true on Android for a sensitive route under the default (sensitive) mode", async () => {
    fn().mockResolvedValue(undefined);
    const { secureAvailable, applySecureForRoute } = createSecureScreen();
    secureAvailable.value = true;
    const ok = await applySecureForRoute(true);
    expect(ok).toBe(true);
    expect(invoke).toHaveBeenCalledWith("plugin:screen-secure|set_secure", {
      secure: true,
    });
  });

  it("applySecureForRoute sets secure=false for a non-sensitive route under sensitive mode", async () => {
    fn().mockResolvedValue(undefined);
    const { secureAvailable, secureScreenMode, applySecureForRoute } =
      createSecureScreen();
    secureAvailable.value = true;
    secureScreenMode.value = "sensitive";
    await applySecureForRoute(false);
    expect(invoke).toHaveBeenCalledWith("plugin:screen-secure|set_secure", {
      secure: false,
    });
  });

  it("applySecureForRoute forces secure=false on every route under off mode (master override)", async () => {
    fn().mockResolvedValue(undefined);
    const { secureAvailable, secureScreenMode, applySecureForRoute } =
      createSecureScreen();
    secureAvailable.value = true;
    secureScreenMode.value = "off";
    await applySecureForRoute(true); // even a sensitive route
    expect(invoke).toHaveBeenCalledWith("plugin:screen-secure|set_secure", {
      secure: false,
    });
  });

  it("applySecureForRoute forces secure=true on every route under always mode", async () => {
    fn().mockResolvedValue(undefined);
    const { secureAvailable, secureScreenMode, applySecureForRoute } =
      createSecureScreen();
    secureAvailable.value = true;
    secureScreenMode.value = "always";
    await applySecureForRoute(false); // even a non-sensitive route
    expect(invoke).toHaveBeenCalledWith("plugin:screen-secure|set_secure", {
      secure: true,
    });
  });

  it("applySecureForRoute returns false when the plugin call rejects on Android (guard aborts)", async () => {
    fn().mockRejectedValue(new Error("bridge"));
    const { secureAvailable, applySecureForRoute } = createSecureScreen();
    secureAvailable.value = true;
    const ok = await applySecureForRoute(true);
    expect(ok).toBe(false);
  });

  it("initSecureScreen loads availability + mode and reconciles the current route", async () => {
    fn().mockImplementation((cmd: string) => {
      if (cmd === "screen_secure_available") return Promise.resolve(true);
      if (cmd === "get_app_config")
        return Promise.resolve({ secure_screen_mode: "off" });
      return Promise.resolve(undefined); // plugin:screen-secure|set_secure
    });
    const { secureAvailable, secureScreenMode, initSecureScreen } =
      createSecureScreen();
    await initSecureScreen();
    expect(secureAvailable.value).toBe(true);
    expect(secureScreenMode.value).toBe("off");
    // Reconcile ran for the default route (non-sensitive) under off → secure=false.
    expect(invoke).toHaveBeenCalledWith("plugin:screen-secure|set_secure", {
      secure: false,
    });
  });

  it("initSecureScreen resolves an unrecognized backend mode (unknown) to sensitive", async () => {
    fn().mockImplementation((cmd: string) => {
      if (cmd === "screen_secure_available") return Promise.resolve(true);
      if (cmd === "get_app_config")
        return Promise.resolve({ secure_screen_mode: "unknown" });
      return Promise.resolve(undefined);
    });
    const { secureScreenMode, initSecureScreen } = createSecureScreen();
    await initSecureScreen();
    expect(secureScreenMode.value).toBe("sensitive");
  });

  it("initSecureScreen is idempotent (availability fetched once)", async () => {
    fn().mockResolvedValue(true);
    const { initSecureScreen } = createSecureScreen();
    await initSecureScreen();
    await initSecureScreen();
    const calls = fn().mock.calls.filter(
      (c) => c[0] === "screen_secure_available",
    );
    expect(calls).toHaveLength(1);
  });

  it("setSecureScreenMode persists the mode and re-applies the current route", async () => {
    fn().mockResolvedValue(undefined);
    const {
      secureAvailable,
      secureScreenMode,
      applySecureForRoute,
      setSecureScreenMode,
    } = createSecureScreen();
    secureAvailable.value = true;
    await applySecureForRoute(true); // current route = sensitive (default mode)
    await setSecureScreenMode("off");
    expect(secureScreenMode.value).toBe("off");
    expect(invoke).toHaveBeenCalledWith("set_secure_screen_mode", {
      mode: "off",
    });
    // Re-applied under off: secure=false.
    expect(invoke).toHaveBeenLastCalledWith("plugin:screen-secure|set_secure", {
      secure: false,
    });
  });

  it("initSecureScreen treats an availability rejection as available (fail-closed), not desktop", async () => {
    fn().mockRejectedValue(new Error("bridge"));
    const { secureAvailable, initSecureScreen } = createSecureScreen();
    await initSecureScreen();
    // A flaky bridge on Android must NOT be mistaken for desktop (fail-open).
    expect(secureAvailable.value).toBe(true);
  });

  it("initSecureScreen keeps the default sensitive mode when get_app_config rejects", async () => {
    fn().mockImplementation((cmd: string) => {
      if (cmd === "screen_secure_available") return Promise.resolve(true);
      if (cmd === "get_app_config")
        return Promise.reject(new Error("pre-setup"));
      return Promise.resolve(undefined); // plugin:screen-secure|set_secure
    });
    const { secureScreenMode, initSecureScreen } = createSecureScreen();
    await initSecureScreen();
    expect(secureScreenMode.value).toBe("sensitive");
  });

  it("setSecureScreenMode reverts the ref and returns false when persistence rejects", async () => {
    fn().mockImplementation((cmd: string) => {
      if (cmd === "set_secure_screen_mode")
        return Promise.reject(new Error("disk"));
      return Promise.resolve(undefined);
    });
    const {
      secureAvailable,
      secureScreenMode,
      applySecureForRoute,
      setSecureScreenMode,
    } = createSecureScreen();
    secureAvailable.value = true;
    await applySecureForRoute(true); // settle a sensitive route (default mode)
    const ok = await setSecureScreenMode("off");
    expect(ok).toBe(false);
    // Reverted to the prior persisted value, so UI/disk/window never desync.
    expect(secureScreenMode.value).toBe("sensitive");
  });

  it("raiseSecureForRoute covers a departing secret page during the transition without committing the route level", async () => {
    fn().mockResolvedValue(undefined);
    const { secureAvailable, applySecureForRoute, raiseSecureForRoute } =
      createSecureScreen();
    secureAvailable.value = true;
    await applySecureForRoute(true); // arrived on a sensitive route
    expect(invoke).toHaveBeenLastCalledWith("plugin:screen-secure|set_secure", {
      secure: true,
    });
    // Leaving sensitive → non-sensitive: raise covers BOTH (still true)…
    await raiseSecureForRoute(true);
    expect(invoke).toHaveBeenLastCalledWith("plugin:screen-secure|set_secure", {
      secure: true,
    });
    // …then settle to the arriving non-sensitive level.
    await applySecureForRoute(false);
    expect(invoke).toHaveBeenLastCalledWith("plugin:screen-secure|set_secure", {
      secure: false,
    });
  });

  it("setSecureOverlay forces FLAG_SECURE on for a non-sensitive route under sensitive mode while the overlay is up", async () => {
    fn().mockResolvedValue(undefined);
    const { secureAvailable, applySecureForRoute, setSecureOverlay } =
      createSecureScreen();
    secureAvailable.value = true;
    await applySecureForRoute(false); // on /entries: not secured
    expect(invoke).toHaveBeenLastCalledWith("plugin:screen-secure|set_secure", {
      secure: false,
    });
    await setSecureOverlay(true); // unlock overlay appears (collects passphrase)
    expect(invoke).toHaveBeenLastCalledWith("plugin:screen-secure|set_secure", {
      secure: true,
    });
    await setSecureOverlay(false); // overlay dismissed → back to route level
    expect(invoke).toHaveBeenLastCalledWith("plugin:screen-secure|set_secure", {
      secure: false,
    });
  });
});
