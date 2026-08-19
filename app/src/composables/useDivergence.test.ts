// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import type { SyncDivergence } from "@/api";
import { useDivergence } from "@/composables/useDivergence";
import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { enableAutoUnmount, flushPromises } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { defineComponent } from "vue";

vi.mock("@tauri-apps/api/core");

enableAutoUnmount(afterEach);

const DIVERGENCE: SyncDivergence = {
  local_ahead: 1,
  remote_ahead: 2,
  remote_tip: "tip-ccc",
  local_only_entries: ["servers/prod"],
  modified_entries: [],
  other_changed_files: [],
};

const PULL_RESULT = {
  changed: true,
  head: "newhead",
  authenticity: {
    mode: "off" as const,
    new_commits: [],
    open_issues: [],
    blocked: false,
  },
};

type Handle = ReturnType<typeof useDivergence>;

describe("useDivergence", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  function mountDivergence(opts?: { unlocked?: boolean }) {
    const onResolved = vi.fn();
    const onPullFfFailed = vi.fn();
    let handle!: Handle;
    const Host = defineComponent({
      setup() {
        handle = useDivergence({
          resolveFailedKey: "common.resolveFailed",
          onResolved,
          onPullFfFailed,
        });
        return () => null;
      },
    });
    const { wrapper, lock, appLock } = mountWithApp(Host, {
      unlocked: opts?.unlocked,
    });
    return { wrapper, lock, appLock, handle, onResolved, onPullFfFailed };
  }

  it("a gate re-lock clears the divergence payload and error", async () => {
    const { appLock, handle } = mountDivergence();
    handle.openDivergence(DIVERGENCE);
    handle.divergeError.value = "boom";
    await flushPromises();

    appLock.setAppLocked(true, "idle");
    await flushPromises();

    expect(handle.divergence.value).toBeNull();
    expect(handle.divergeError.value).toBe("");
  });

  it("an identity hard lock clears the divergence payload too (onAnyLock)", async () => {
    const { lock, handle } = mountDivergence();
    handle.openDivergence(DIVERGENCE);
    await flushPromises();

    lock.setLocked(true);
    await flushPromises();

    expect(handle.divergence.value).toBeNull();
  });

  it("a gate re-lock cancels a parked keep-mine resolve — AUTH_CANCELLED swallowed, no publish", async () => {
    // Pins the parked-frame fix (issue #20 review): resolveDivergence captures
    // its args, then runWithAuth PARKS on the per-op auth overlay while the
    // identity is uncached. A gate lock must cancel the parked caller
    // (cancelAuth → AUTH_CANCELLED) so the resolve never silently runs after
    // the unlock — the modal is gone by then.
    vi.mocked(invoke).mockResolvedValue(PULL_RESULT);
    // unlocked:false → identity NOT cached → runWithAuth parks instead of running.
    const { appLock, handle } = mountDivergence({ unlocked: false });
    handle.openDivergence(DIVERGENCE);
    await flushPromises();

    const parked = handle.resolveDivergence("keep_mine");
    await flushPromises();

    appLock.setAppLocked(true, "idle"); // gate lock → cancelAuth rejects parked
    await parked; // must resolve (cancelled), not reject
    await flushPromises();

    expect(
      vi
        .mocked(invoke)
        .mock.calls.filter((c) => c[0] === "resolve_sync_divergence"),
    ).toHaveLength(0);
    expect(handle.divergence.value).toBeNull();
    expect(handle.resolving.value).toBe(false);
    expect(handle.divergeError.value).toBe(""); // AUTH_CANCELLED swallowed
  });

  it("an identity hard lock cancels a parked keep-mine resolve too (onAnyLock's other half)", async () => {
    // The identity-edge sibling of the gate test above, in its realistic
    // Immediate-mode sequence: unlocked session, identity soft-wiped post-op
    // (parking the resolve), then the hard lock — a real false→true edge that
    // both fires onAnyLock and (centrally, in useLockState) cancels parked
    // auths. ({unlocked:false} mounts start already-locked, where setLocked
    // early-returns — no edge.)
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "get_auth_state")
        return Promise.resolve({
          configured: true,
          encrypted: true,
          unlocked: true,
          identity_type: "x25519",
        });
      return Promise.resolve(PULL_RESULT);
    });
    const { lock, handle } = mountDivergence({ unlocked: false });
    await lock.init(); // arms the listener + unlocks (mocked auth above)
    const handler = vi
      .mocked(listen)
      .mock.calls.find((c) => c[0] === "identity-lock-state")?.[1] as (e: {
      payload: { locked: boolean; soft?: boolean };
    }) => void;
    handler({ payload: { locked: true, soft: true } }); // post-op soft wipe

    handle.openDivergence(DIVERGENCE);
    await flushPromises();
    const parked = handle.resolveDivergence("keep_mine");
    await flushPromises();

    lock.setLocked(true); // identity hard-lock edge
    await parked; // resolves (cancelled), not rejects
    await flushPromises();

    expect(
      vi
        .mocked(invoke)
        .mock.calls.filter((c) => c[0] === "resolve_sync_divergence"),
    ).toHaveLength(0);
    expect(handle.resolving.value).toBe(false);
  });
});
