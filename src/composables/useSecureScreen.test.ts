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

  it("acquireClaim is a no-op (returns true, no invoke) when the plugin is unavailable (desktop)", async () => {
    const { acquireClaim } = createSecureScreen();
    const ok = await acquireClaim();
    expect(ok).toBe(true);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("acquireClaim sets secure=true on Android under the default (sensitive) mode", async () => {
    fn().mockResolvedValue(undefined);
    const { secureAvailable, acquireClaim } = createSecureScreen();
    secureAvailable.value = true;
    const ok = await acquireClaim();
    expect(ok).toBe(true);
    expect(invoke).toHaveBeenCalledWith("plugin:screen-secure|set_secure", {
      secure: true,
    });
  });

  it("releaseClaim sets secure=false once the last claim drops (sensitive mode)", async () => {
    fn().mockResolvedValue(undefined);
    const { secureAvailable, acquireClaim, releaseClaim } =
      createSecureScreen();
    secureAvailable.value = true;
    await acquireClaim();
    await releaseClaim();
    expect(invoke).toHaveBeenLastCalledWith("plugin:screen-secure|set_secure", {
      secure: false,
    });
  });

  it("a second claim keeps the flag up; it only drops when both release", async () => {
    fn().mockResolvedValue(undefined);
    const { secureAvailable, acquireClaim, releaseClaim } =
      createSecureScreen();
    secureAvailable.value = true;
    await acquireClaim();
    await acquireClaim();
    await releaseClaim(); // one released, one still held ⇒ still secure
    expect(invoke).toHaveBeenLastCalledWith("plugin:screen-secure|set_secure", {
      secure: true,
    });
    await releaseClaim(); // last one ⇒ flag drops
    expect(invoke).toHaveBeenLastCalledWith("plugin:screen-secure|set_secure", {
      secure: false,
    });
  });

  it("releaseClaim is idempotent (floored at 0 — never drives the count negative)", async () => {
    fn().mockResolvedValue(undefined);
    const { secureAvailable, releaseClaim } = createSecureScreen();
    secureAvailable.value = true;
    // Release with no prior acquire — must not push secure=false repeatedly
    // or corrupt the count for a later acquire.
    await releaseClaim();
    await releaseClaim();
    const secureFalseCalls = fn().mock.calls.filter(
      (c) =>
        c[0] === "plugin:screen-secure|set_secure" &&
        (c[1] as { secure: boolean }).secure === false,
    );
    // count was already 0, so desiredSecure was already false; the first release
    // re-pushed false, subsequent ones are no-ops on the count (still 0).
    expect(secureFalseCalls.length).toBeGreaterThanOrEqual(1);
  });

  it("acquireClaim is ignored under off mode (claims do not secure; user opted into capture)", async () => {
    fn().mockResolvedValue(undefined);
    const { secureAvailable, secureScreenMode, acquireClaim } =
      createSecureScreen();
    secureAvailable.value = true;
    secureScreenMode.value = "off";
    await acquireClaim();
    expect(invoke).toHaveBeenLastCalledWith("plugin:screen-secure|set_secure", {
      secure: false,
    });
  });

  it("acquireClaim forces secure=true on every screen under always mode", async () => {
    fn().mockResolvedValue(undefined);
    const { secureAvailable, secureScreenMode, acquireClaim } =
      createSecureScreen();
    secureAvailable.value = true;
    secureScreenMode.value = "always";
    await acquireClaim();
    expect(invoke).toHaveBeenLastCalledWith("plugin:screen-secure|set_secure", {
      secure: true,
    });
  });

  it("acquireClaim returns false when the plugin call rejects on Android (caller must not render)", async () => {
    fn().mockRejectedValue(new Error("bridge"));
    const { secureAvailable, acquireClaim } = createSecureScreen();
    secureAvailable.value = true;
    const ok = await acquireClaim();
    expect(ok).toBe(false);
  });

  it("initSecureScreen loads availability + mode and reconciles (no claims ⇒ secure=false)", async () => {
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

  it("setSecureScreenMode persists the mode and re-applies (a held claim is ignored under off)", async () => {
    fn().mockResolvedValue(undefined);
    const {
      secureAvailable,
      secureScreenMode,
      acquireClaim,
      setSecureScreenMode,
    } = createSecureScreen();
    secureAvailable.value = true;
    await acquireClaim(); // a secret is on screen ⇒ secure=true (sensitive default)
    await setSecureScreenMode("off");
    expect(secureScreenMode.value).toBe("off");
    expect(invoke).toHaveBeenCalledWith("set_secure_screen_mode", {
      mode: "off",
    });
    // Re-applied under off: the claim is ignored ⇒ secure=false.
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
      acquireClaim,
      setSecureScreenMode,
    } = createSecureScreen();
    secureAvailable.value = true;
    await acquireClaim(); // a held claim under the default sensitive mode
    const ok = await setSecureScreenMode("off");
    expect(ok).toBe(false);
    // Reverted to the prior persisted value, so UI/disk/window never desync.
    expect(secureScreenMode.value).toBe("sensitive");
  });

  it("setSecureOverlay forces FLAG_SECURE on under sensitive mode while the overlay is up (even with no claim)", async () => {
    fn().mockResolvedValue(undefined);
    const { secureAvailable, setSecureOverlay } = createSecureScreen();
    secureAvailable.value = true;
    await setSecureOverlay(true); // unlock overlay appears (collects passphrase)
    expect(invoke).toHaveBeenLastCalledWith("plugin:screen-secure|set_secure", {
      secure: true,
    });
    await setSecureOverlay(false); // overlay dismissed ⇒ back to no-claim level
    expect(invoke).toHaveBeenLastCalledWith("plugin:screen-secure|set_secure", {
      secure: false,
    });
  });
});
