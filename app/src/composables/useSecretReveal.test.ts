// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import {
  createLockState,
  createSecureScreen,
  createSecuritySettings,
  LOCK_KEY,
  SECURE_SCREEN_KEY,
  SECURITY_SETTINGS_KEY,
} from "@/composables";
import { withSetup } from "@/test/withSetup";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { App } from "vue";
import { useSecretReveal } from "./useSecretReveal";

vi.mock("@tauri-apps/api/core");

const fn = () => invoke as ReturnType<typeof vi.fn>;
const SECURE_TRUE = ["plugin:screen-secure|set_secure", { secure: true }];
const SECURE_FALSE = ["plugin:screen-secure|set_secure", { secure: false }];

describe("useSecretReveal", () => {
  let app: App;
  beforeEach(() => {
    vi.clearAllMocks();
  });
  afterEach(() => {
    // Unmount fires useWipeOnLeave(clear) → cancels the auto-clear timer and
    // releases the claim, so no dangling timer leaks between tests.
    app.unmount();
  });

  function setupReveal(available = true) {
    const [r, a] = withSetup(
      () => useSecretReveal(),
      (app) => {
        app.provide(LOCK_KEY, createLockState({ unlocked: true }));
        app.provide(SECURE_SCREEN_KEY, createSecureScreen({ available }));
        app.provide(SECURITY_SETTINGS_KEY, createSecuritySettings());
      },
    );
    app = a;
    return r;
  }

  it("acquires FLAG_SECURE before the secret is shown, then releases on clear", async () => {
    fn().mockResolvedValue(undefined);
    const r = setupReveal(true);

    // The secret must come through `withClaim`, which raises FLAG_SECURE first.
    const claimed = await r.withClaim(async () => ({
      password: "s3cret",
      notes: "n",
    }));
    expect(claimed).not.toBeNull();
    // Flag is up…
    expect(invoke).toHaveBeenCalledWith(...SECURE_TRUE);
    // …but the secret is NOT on screen yet (raise-before-render).
    expect(r.password.value).toBeNull();

    r.reveal(claimed!);
    expect(r.password.value).toBe("s3cret");

    r.clear();
    expect(r.password.value).toBeNull();
    // clear() released the claim ⇒ FLAG_SECURE off.
    expect(invoke).toHaveBeenLastCalledWith(...SECURE_FALSE);
  });

  it("clear is idempotent across the wipe double-fire (popstate then unmount)", async () => {
    fn().mockResolvedValue(undefined);
    const r = setupReveal(true);
    const claimed = await r.withClaim(async () => ({
      password: "s3cret",
      notes: "",
    }));
    expect(claimed).not.toBeNull();
    r.reveal(claimed!);
    r.clear(); // popstate wipe
    r.clear(); // unmount wipe — must not drive the count negative
    await flushPromises();
    // The afterEach's app.unmount() triggers a third clear(); all are safe.
    expect(r.password.value).toBeNull();
  });
});
