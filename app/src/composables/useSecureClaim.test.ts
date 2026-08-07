// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { withSetup } from "@/test/withSetup";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSecureClaim } from "./useSecureClaim";
import { createSecureScreen, SECURE_SCREEN_KEY } from "./useSecureScreen";

vi.mock("@tauri-apps/api/core");

const fn = () => invoke as ReturnType<typeof vi.fn>;

/** Mount `useSecureClaim` under a fresh secure-screen singleton. */
function setupClaim(available = true) {
  return withSetup(
    () => useSecureClaim(),
    (app) => app.provide(SECURE_SCREEN_KEY, createSecureScreen({ available })),
  );
}

const SECURE_TRUE = ["plugin:screen-secure|set_secure", { secure: true }];
const SECURE_FALSE = ["plugin:screen-secure|set_secure", { secure: false }];

describe("useSecureClaim", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("withClaim raises FLAG_SECURE and returns the (transparently branded) result", async () => {
    fn().mockResolvedValue(undefined);
    const [claim] = setupClaim(true);
    const result = await claim.withClaim(async () => "the-secret");
    // The brand is compile-time only; at runtime the value passes through.
    expect(result).toBe("the-secret");
    expect(invoke).toHaveBeenCalledWith(...SECURE_TRUE);
  });

  it("withClaim returns null and never runs op when the acquire IPC fails", async () => {
    // The per-op abort: a failed acquire must NOT render the secret.
    fn().mockRejectedValue(new Error("bridge"));
    const op = vi.fn().mockResolvedValue("secret");
    const [claim] = setupClaim(true);
    const result = await claim.withClaim(op);
    expect(result).toBeNull();
    expect(op).not.toHaveBeenCalled();
  });

  it("is a no-op (no IPC) on desktop where the plugin is unavailable", async () => {
    const op = vi.fn().mockResolvedValue("secret");
    const [claim] = setupClaim(false); // desktop
    const result = await claim.withClaim(op);
    expect(result).toBe("secret");
    expect(invoke).not.toHaveBeenCalled();
  });

  it("releases the claim (FLAG_SECURE off) when the scope unmounts", async () => {
    fn().mockResolvedValue(undefined);
    const [claim, app] = setupClaim(true);
    await claim.withClaim(async () => "x"); // acquire
    app.unmount(); // onScopeDispose → release
    await flushPromises();
    expect(invoke).toHaveBeenLastCalledWith(...SECURE_FALSE);
  });

  it("withClaim releases the claim and rethrows if op throws (no stranded claim)", async () => {
    fn().mockResolvedValue(undefined);
    const [claim] = setupClaim(true);
    await expect(
      claim.withClaim(async () => {
        throw new Error("decrypt-failed");
      }),
    ).rejects.toThrow("decrypt-failed");
    // Acquired then released ⇒ back to count 0 ⇒ FLAG_SECURE off.
    expect(invoke).toHaveBeenLastCalledWith(...SECURE_FALSE);
  });

  it("release is idempotent — a release with no held claim can't drive the count negative", async () => {
    fn().mockResolvedValue(undefined);
    const [claim] = setupClaim(true);
    // Release with nothing held — must not corrupt the counter.
    claim.release();
    claim.release();
    // A subsequent acquire+release still pairs cleanly (count wasn't negatived).
    const ok = await claim.acquire();
    expect(ok).toBe(true);
    claim.release();
    // Count is 0 again; one more release is a no-op, not a negative.
    claim.release();
    const ok2 = await claim.acquire();
    expect(ok2).toBe(true);
  });
});
