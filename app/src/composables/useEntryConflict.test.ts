// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import type { SecretParts } from "@/api";
import {
  useEntryConflict,
  type EntryConflictPayload,
} from "@/composables/useEntryConflict";
import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { enableAutoUnmount, flushPromises } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { defineComponent } from "vue";

vi.mock("@tauri-apps/api/core");

// Mount a host component whose setup() runs useEntryConflict under the
// app-shell provide block — the composable injects useLockState() + useI18n()
// internally, so it must run inside a real component instance with all the
// shell keys. The handle is captured into an outer variable (its refs are
// reactive, so reading .value works the same); setup returns a null render
// function so Vue doesn't warn about a missing template.
enableAutoUnmount(afterEach);

const PAYLOAD: EntryConflictPayload = {
  name: "servers/prod",
  base_oid: "base-aaa",
  current_oid: "curr-bbb",
  remote_tip: "tip-ccc",
  op: "edit",
};

/** Sample structured parts used as the captured edit payload across tests. */
const PARTS: SecretParts = {
  password: "my-pw",
  attributes: [],
  body: "my-body",
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

type Handle = ReturnType<typeof useEntryConflict>;

describe("useEntryConflict", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  function mountConflict(opts?: { unlocked?: boolean }) {
    const onResolved = vi.fn();
    const onPullFfFailed = vi.fn();
    const onAuthenticityBlocked = vi.fn();
    let handle!: Handle;
    const Host = defineComponent({
      setup() {
        handle = useEntryConflict({
          resolveFailedKey: "entry.resolveFailed",
          onResolved,
          onPullFfFailed,
          onAuthenticityBlocked,
        });
        return () => null;
      },
    });
    const { wrapper, lock, appLock } = mountWithApp(Host, {
      unlocked: opts?.unlocked,
    });
    return {
      wrapper,
      lock,
      appLock,
      handle,
      onResolved,
      onPullFfFailed,
      onAuthenticityBlocked,
    };
  }

  it("keep_mine edit re-sends the captured pendingBody as content via resolve_entry_conflict (runWithAuth path)", async () => {
    vi.mocked(invoke).mockResolvedValue(PULL_RESULT);
    const { handle, onResolved } = mountConflict();
    handle.openConflict(PAYLOAD, PARTS);
    await flushPromises();

    await handle.resolveConflict("keep_mine");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("resolve_entry_conflict", {
      parts: PARTS,
      op: "edit",
      choice: "keep_mine",
      expectedRemoteOid: PAYLOAD.remote_tip,
      name: PAYLOAD.name,
    });
    expect(onResolved).toHaveBeenCalledWith(PULL_RESULT, "keep_mine", "edit");
  });

  it("keep_mine create re-sends the captured body and routes through runWithAuth (op: create)", async () => {
    // Create keep-mine is identity-gated just like edit (it re-encrypts the new
    // body), and re-sends the captured pendingBody as content.
    vi.mocked(invoke).mockResolvedValue(PULL_RESULT);
    const { handle, onResolved } = mountConflict();
    handle.openConflict({ ...PAYLOAD, op: "create" }, PARTS);
    await flushPromises();

    await handle.resolveConflict("keep_mine");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("resolve_entry_conflict", {
      parts: PARTS,
      op: "create",
      choice: "keep_mine",
      expectedRemoteOid: PAYLOAD.remote_tip,
      name: PAYLOAD.name,
    });
    expect(onResolved).toHaveBeenCalledWith(PULL_RESULT, "keep_mine", "create");
  });

  it("keep_theirs passes content:null (no runWithAuth gating)", async () => {
    vi.mocked(invoke).mockResolvedValue(PULL_RESULT);
    const { handle } = mountConflict();
    handle.openConflict(PAYLOAD, PARTS);
    await flushPromises();

    await handle.resolveConflict("keep_theirs");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("resolve_entry_conflict", {
      parts: null,
      op: "edit",
      choice: "keep_theirs",
      expectedRemoteOid: PAYLOAD.remote_tip,
      name: PAYLOAD.name,
    });
  });

  it("PULL_FF_FAILED rejection drops the modal (conflict → null) and fires onPullFfFailed", async () => {
    vi.mocked(invoke).mockRejectedValue({
      code: "PULL_FF_FAILED",
      message: "fast-forward failed",
    });
    const { handle, onPullFfFailed } = mountConflict();
    handle.openConflict(PAYLOAD, PARTS);
    await flushPromises();
    // toEqual (deep) — Vue reactively proxies the stored payload.
    expect(handle.conflict.value).toEqual(PAYLOAD);

    await handle.resolveConflict("keep_mine");
    await flushPromises();

    expect(onPullFfFailed).toHaveBeenCalledTimes(1);
    expect(handle.conflict.value).toBeNull();
  });

  it("non-PULL_FF_FAILED error sets conflictError (resolveFailedKey fallback) and keeps the modal", async () => {
    vi.mocked(invoke).mockRejectedValue({
      code: "STORE_ERROR",
      message: "Disk full",
    });
    const { handle } = mountConflict();
    handle.openConflict(PAYLOAD, PARTS);
    await flushPromises();

    await handle.resolveConflict("keep_mine");
    await flushPromises();

    expect(handle.conflict.value).toEqual(PAYLOAD);
    expect(handle.conflictError.value).toBe("Disk full");
  });

  it("cancelConflict invokes discard_divergence and clears the conflict", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);
    const { handle } = mountConflict();
    handle.openConflict(PAYLOAD, PARTS);
    await flushPromises();

    handle.cancelConflict();
    await flushPromises();

    expect(handle.conflict.value).toBeNull();
    expect(invoke).toHaveBeenCalledWith("discard_divergence");
  });

  it("hard lock wipes pendingBody — a subsequent keep_mine edit sends content:null", async () => {
    // Pins the fix: onLock must null the captured plaintext, not just the
    // modal payload. Otherwise the closure holds a second copy of the secret
    // that survives the lock. Asserted indirectly: after a lock + re-unlock
    // (so runWithAuth stops parking), re-raising the conflict WITHOUT a fresh
    // body (the page's draft was wiped too) and picking keep_mine must send
    // content:null — if pendingBody had survived, it would send "my-body".
    vi.mocked(invoke).mockResolvedValue(PULL_RESULT);
    const { lock, handle } = mountConflict();
    handle.openConflict(PAYLOAD, PARTS);
    await flushPromises();

    // Hard lock fires onLock → clears conflict + pendingBody and flips
    // identityCached to false (mirrors a real idle/manual lock).
    lock.setLocked(true);
    await flushPromises();
    expect(handle.conflict.value).toBeNull();

    // Re-cache the identity so runWithAuth runs the op instead of parking on
    // the per-op auth overlay (no App.vue init() in page-style tests to release
    // parked waiters). The page's own wipe would normally tear everything down
    // on a real lock; we keep the host alive to probe pendingBody's state.
    lock.setLocked(false);
    await flushPromises();

    // Re-raise the same conflict WITHOUT recapturing a body: assign the ref
    // directly so openConflict (which would reset pendingBody) is bypassed.
    handle.conflict.value = PAYLOAD;
    await flushPromises();

    await handle.resolveConflict("keep_mine");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("resolve_entry_conflict", {
      parts: null,
      op: "edit",
      choice: "keep_mine",
      expectedRemoteOid: PAYLOAD.remote_tip,
      name: PAYLOAD.name,
    });
  });

  it("gate re-lock wipes pendingParts — a subsequent keep_mine edit sends content:null (onAnyLock)", async () => {
    // The gate-driven sibling of the hard-lock test above: a gate re-lock
    // fires onAnyLock, which must clear the conflict AND the captured
    // plaintext (issue #20 — the gate mask covers the page but does not
    // unmount it).
    vi.mocked(invoke).mockResolvedValue(PULL_RESULT);
    const { appLock, lock, handle } = mountConflict();
    handle.openConflict(PAYLOAD, PARTS);
    await flushPromises();

    appLock.setAppLocked(true, "idle");
    await flushPromises();
    expect(handle.conflict.value).toBeNull();

    // Re-cache the identity so runWithAuth runs instead of parking, then
    // re-raise the conflict WITHOUT recapturing parts (mirrors the onLock
    // test) — a surviving pendingParts would send PARTS, not null.
    lock.setLocked(false);
    await flushPromises();
    handle.conflict.value = PAYLOAD;
    await flushPromises();

    await handle.resolveConflict("keep_mine");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("resolve_entry_conflict", {
      parts: null,
      op: "edit",
      choice: "keep_mine",
      expectedRemoteOid: PAYLOAD.remote_tip,
      name: PAYLOAD.name,
    });
  });

  it("gate re-lock cancels a parked keep-mine resolve — AUTH_CANCELLED swallowed, no publish", async () => {
    // Pins the parked-frame fix: resolveConflict captures `parts` into its
    // local frame BEFORE runWithAuth parks it (identity uncached), so a gate
    // lock must cancelAuth the parked caller — otherwise the frame rides the
    // lock window holding the plaintext and silently publishes after unlock
    // with the modal already gone.
    vi.mocked(invoke).mockResolvedValue(PULL_RESULT);
    // unlocked:false → identity NOT cached → runWithAuth parks.
    const { appLock, handle } = mountConflict({ unlocked: false });
    handle.openConflict(PAYLOAD, PARTS);
    await flushPromises();

    const parked = handle.resolveConflict("keep_mine");
    await flushPromises();

    appLock.setAppLocked(true, "idle"); // gate lock → cancelAuth rejects parked
    await parked; // must resolve (cancelled), not reject
    await flushPromises();

    expect(
      vi
        .mocked(invoke)
        .mock.calls.filter((c) => c[0] === "resolve_entry_conflict"),
    ).toHaveLength(0);
    expect(handle.conflict.value).toBeNull();
    expect(handle.resolving.value).toBe(false);
    expect(handle.conflictError.value).toBe("");
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
    const { lock, handle } = mountConflict({ unlocked: false });
    await lock.init(); // arms the listener + unlocks (mocked auth above)
    const handler = vi
      .mocked(listen)
      .mock.calls.find((c) => c[0] === "identity-lock-state")?.[1] as (e: {
      payload: { locked: boolean; soft?: boolean };
    }) => void;
    handler({ payload: { locked: true, soft: true } }); // post-op soft wipe

    handle.openConflict(PAYLOAD, PARTS);
    await flushPromises();
    const parked = handle.resolveConflict("keep_mine");
    await flushPromises();

    lock.setLocked(true); // identity hard-lock edge
    await parked; // resolves (cancelled), not rejects
    await flushPromises();

    expect(
      vi
        .mocked(invoke)
        .mock.calls.filter((c) => c[0] === "resolve_entry_conflict"),
    ).toHaveLength(0);
    expect(handle.resolving.value).toBe(false);
  });

  it("an authenticity-blocked resolve routes to onAuthenticityBlocked, not onResolved (no false 'saved')", async () => {
    // Pins the R026 #1 fix: resolve_entry_conflict returns Ok with
    // authenticity.blocked=true (Enforce refused the re-fetch — nothing
    // committed, HEAD unchanged). The composable must NOT call onResolved
    // (which the page uses to toast success); it routes to onAuthenticityBlocked
    // so the page surfaces the block instead.
    vi.mocked(invoke).mockResolvedValue({
      ...PULL_RESULT,
      authenticity: { ...PULL_RESULT.authenticity, blocked: true },
    });
    const { handle, onResolved, onAuthenticityBlocked } = mountConflict();
    handle.openConflict(PAYLOAD, PARTS);
    await flushPromises();

    await handle.resolveConflict("keep_mine");
    await flushPromises();

    expect(onAuthenticityBlocked).toHaveBeenCalledTimes(1);
    expect(onResolved).not.toHaveBeenCalled();
    expect(handle.conflict.value).toBeNull();
  });

  it("unmount wipes pendingBody — a subsequent keep_mine edit sends content:null (unmount window)", async () => {
    // Pins the second hook (onBeforeUnmount): a route-away while the modal is
    // open must null the captured plaintext. Asserted indirectly as the onLock
    // test — after unmount + a re-raised conflict WITHOUT recapturing a body,
    // keep_mine edit sends content:null (a surviving pendingBody would send
    // "my-body"). The onLock test above covers the hard-lock window; this covers
    // the unmount window onLock does not fire on.
    vi.mocked(invoke).mockResolvedValue(PULL_RESULT);
    const { wrapper, lock, handle } = mountConflict();
    handle.openConflict(PAYLOAD, PARTS);
    await flushPromises();

    // Unmount fires onBeforeUnmount → nulls pendingBody (the unmount half).
    wrapper.unmount();
    await flushPromises();

    // Re-cache the identity (mirrors a fresh mount) so runWithAuth runs the op
    // instead of parking; then re-raise the conflict WITHOUT recapturing a body.
    lock.setLocked(false);
    await flushPromises();
    handle.conflict.value = PAYLOAD;
    await flushPromises();

    await handle.resolveConflict("keep_mine");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("resolve_entry_conflict", {
      parts: null,
      op: "edit",
      choice: "keep_mine",
      expectedRemoteOid: PAYLOAD.remote_tip,
      name: PAYLOAD.name,
    });
  });
});
