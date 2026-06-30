// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect, vi, beforeEach } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import {
  useSecureScreen,
  __resetSecureScreenForTests,
} from "./useSecureScreen";

vi.mock("@tauri-apps/api/core");

const fn = () => invoke as ReturnType<typeof vi.fn>;

describe("useSecureScreen", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    __resetSecureScreenForTests();
  });

  it("applySecureForRoute is a no-op (returns true, no invoke) when the plugin is unavailable (desktop)", async () => {
    const { applySecureForRoute } = useSecureScreen();
    const ok = await applySecureForRoute(true);
    expect(ok).toBe(true);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("applySecureForRoute sets secure=true on Android for a sensitive route with the toggle ON", async () => {
    fn().mockResolvedValue(undefined);
    const { secureAvailable, applySecureForRoute } = useSecureScreen();
    secureAvailable.value = true;
    const ok = await applySecureForRoute(true);
    expect(ok).toBe(true);
    expect(invoke).toHaveBeenCalledWith("plugin:screen-secure|set_secure", {
      secure: true,
    });
  });

  it("applySecureForRoute sets secure=false for a non-sensitive route", async () => {
    fn().mockResolvedValue(undefined);
    const { secureAvailable, secureScreen, applySecureForRoute } =
      useSecureScreen();
    secureAvailable.value = true;
    secureScreen.value = true;
    await applySecureForRoute(false);
    expect(invoke).toHaveBeenCalledWith("plugin:screen-secure|set_secure", {
      secure: false,
    });
  });

  it("applySecureForRoute sets secure=false on a sensitive route when the toggle is OFF (master override)", async () => {
    fn().mockResolvedValue(undefined);
    const { secureAvailable, secureScreen, applySecureForRoute } =
      useSecureScreen();
    secureAvailable.value = true;
    secureScreen.value = false;
    await applySecureForRoute(true);
    expect(invoke).toHaveBeenCalledWith("plugin:screen-secure|set_secure", {
      secure: false,
    });
  });

  it("applySecureForRoute returns false when the plugin call rejects on Android (guard aborts)", async () => {
    fn().mockRejectedValue(new Error("bridge"));
    const { secureAvailable, applySecureForRoute } = useSecureScreen();
    secureAvailable.value = true;
    const ok = await applySecureForRoute(true);
    expect(ok).toBe(false);
  });

  it("initSecureScreen loads availability + toggle and reconciles the current route", async () => {
    fn().mockImplementation((cmd: string) => {
      if (cmd === "screen_secure_available") return Promise.resolve(true);
      if (cmd === "get_app_config")
        return Promise.resolve({ secure_screen: false });
      return Promise.resolve(undefined); // plugin:screen-secure|set_secure
    });
    const { secureAvailable, secureScreen, initSecureScreen } =
      useSecureScreen();
    await initSecureScreen();
    expect(secureAvailable.value).toBe(true);
    expect(secureScreen.value).toBe(false);
    // Reconcile ran for the default route (non-sensitive) → shouldSecure=false.
    expect(invoke).toHaveBeenCalledWith("plugin:screen-secure|set_secure", {
      secure: false,
    });
  });

  it("initSecureScreen is idempotent (availability fetched once)", async () => {
    fn().mockResolvedValue(true);
    const { initSecureScreen } = useSecureScreen();
    await initSecureScreen();
    await initSecureScreen();
    const calls = fn().mock.calls.filter(
      (c) => c[0] === "screen_secure_available",
    );
    expect(calls).toHaveLength(1);
  });

  it("setSecureScreen persists the toggle and re-applies the current route", async () => {
    fn().mockResolvedValue(undefined);
    const {
      secureAvailable,
      secureScreen,
      applySecureForRoute,
      setSecureScreen,
    } = useSecureScreen();
    secureAvailable.value = true;
    await applySecureForRoute(true); // current route = sensitive
    await setSecureScreen(false);
    expect(secureScreen.value).toBe(false);
    expect(invoke).toHaveBeenCalledWith("set_secure_screen", {
      enabled: false,
    });
    // Re-applied with the new toggle: shouldSecure = false → secure=false.
    expect(invoke).toHaveBeenLastCalledWith("plugin:screen-secure|set_secure", {
      secure: false,
    });
  });

  it("initSecureScreen treats an availability rejection as available (fail-closed), not desktop", async () => {
    fn().mockRejectedValue(new Error("bridge"));
    const { secureAvailable, initSecureScreen } = useSecureScreen();
    await initSecureScreen();
    // A flaky bridge on Android must NOT be mistaken for desktop (fail-open).
    expect(secureAvailable.value).toBe(true);
  });

  it("initSecureScreen keeps the default ON when get_app_config rejects", async () => {
    fn().mockImplementation((cmd: string) => {
      if (cmd === "screen_secure_available") return Promise.resolve(true);
      if (cmd === "get_app_config")
        return Promise.reject(new Error("pre-setup"));
      return Promise.resolve(undefined); // plugin:screen-secure|set_secure
    });
    const { secureScreen, initSecureScreen } = useSecureScreen();
    await initSecureScreen();
    expect(secureScreen.value).toBe(true);
  });

  it("setSecureScreen reverts the ref and returns false when persistence rejects", async () => {
    fn().mockImplementation((cmd: string) => {
      if (cmd === "set_secure_screen") return Promise.reject(new Error("disk"));
      return Promise.resolve(undefined);
    });
    const {
      secureAvailable,
      secureScreen,
      applySecureForRoute,
      setSecureScreen,
    } = useSecureScreen();
    secureAvailable.value = true;
    await applySecureForRoute(true); // settle a sensitive route (toggle ON)
    const ok = await setSecureScreen(false);
    expect(ok).toBe(false);
    // Reverted to the prior persisted value, so UI/disk/window never desync.
    expect(secureScreen.value).toBe(true);
  });

  it("raiseSecureForRoute covers a departing secret page during the transition without committing the route level", async () => {
    fn().mockResolvedValue(undefined);
    const { secureAvailable, applySecureForRoute, raiseSecureForRoute } =
      useSecureScreen();
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

  it("setSecureOverlay forces FLAG_SECURE on for a non-sensitive route while the overlay is up", async () => {
    fn().mockResolvedValue(undefined);
    const { secureAvailable, applySecureForRoute, setSecureOverlay } =
      useSecureScreen();
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
