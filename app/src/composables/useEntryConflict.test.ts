// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import {
  useEntryConflict,
  type EntryConflictPayload,
} from "@/composables/useEntryConflict";
import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
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

  function mountConflict() {
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
    const { wrapper, lock } = mountWithApp(Host);
    return {
      wrapper,
      lock,
      handle,
      onResolved,
      onPullFfFailed,
      onAuthenticityBlocked,
    };
  }

  it("keep_mine edit re-sends the captured pendingBody as content via resolve_entry_conflict (runWithAuth path)", async () => {
    vi.mocked(invoke).mockResolvedValue(PULL_RESULT);
    const { handle, onResolved } = mountConflict();
    handle.openConflict(PAYLOAD, "my-body");
    await flushPromises();

    await handle.resolveConflict("keep_mine");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("resolve_entry_conflict", {
      content: "my-body",
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
    handle.openConflict({ ...PAYLOAD, op: "create" }, "my-new-body");
    await flushPromises();

    await handle.resolveConflict("keep_mine");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("resolve_entry_conflict", {
      content: "my-new-body",
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
    handle.openConflict(PAYLOAD, "my-body");
    await flushPromises();

    await handle.resolveConflict("keep_theirs");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("resolve_entry_conflict", {
      content: null,
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
    handle.openConflict(PAYLOAD, "my-body");
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
    handle.openConflict(PAYLOAD, "my-body");
    await flushPromises();

    await handle.resolveConflict("keep_mine");
    await flushPromises();

    expect(handle.conflict.value).toEqual(PAYLOAD);
    expect(handle.conflictError.value).toBe("Disk full");
  });

  it("cancelConflict invokes discard_divergence and clears the conflict", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);
    const { handle } = mountConflict();
    handle.openConflict(PAYLOAD, "my-body");
    await flushPromises();

    handle.cancelConflict();
    await flushPromises();

    expect(handle.conflict.value).toBeNull();
    expect(invoke).toHaveBeenCalledWith("discard_divergence");
  });

  it("hard lock wipes pendingBody — a subsequent keep_mine edit sends content:null (F1)", async () => {
    // Pins the F1 fix: onLock must null the captured plaintext, not just the
    // modal payload. Otherwise the closure holds a second copy of the secret
    // that survives the lock. Asserted indirectly: after a lock + re-unlock
    // (so runWithAuth stops parking), re-raising the conflict WITHOUT a fresh
    // body (the page's draft was wiped too) and picking keep_mine must send
    // content:null — if pendingBody had survived, it would send "my-body".
    vi.mocked(invoke).mockResolvedValue(PULL_RESULT);
    const { lock, handle } = mountConflict();
    handle.openConflict(PAYLOAD, "my-body");
    await flushPromises();

    // Hard lock fires onLock → clears conflict + pendingBody (F1) and flips
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
      content: null,
      op: "edit",
      choice: "keep_mine",
      expectedRemoteOid: PAYLOAD.remote_tip,
      name: PAYLOAD.name,
    });
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
    handle.openConflict(PAYLOAD, "my-body");
    await flushPromises();

    await handle.resolveConflict("keep_mine");
    await flushPromises();

    expect(onAuthenticityBlocked).toHaveBeenCalledTimes(1);
    expect(onResolved).not.toHaveBeenCalled();
    expect(handle.conflict.value).toBeNull();
  });

  it("unmount wipes pendingBody — a subsequent keep_mine edit sends content:null (F1 unmount window)", async () => {
    // Pins the second F1 hook (onBeforeUnmount): a route-away while the modal is
    // open must null the captured plaintext. Asserted indirectly as the onLock
    // test — after unmount + a re-raised conflict WITHOUT recapturing a body,
    // keep_mine edit sends content:null (a surviving pendingBody would send
    // "my-body"). The onLock test above covers the hard-lock window; this covers
    // the unmount window onLock does not fire on.
    vi.mocked(invoke).mockResolvedValue(PULL_RESULT);
    const { wrapper, lock, handle } = mountConflict();
    handle.openConflict(PAYLOAD, "my-body");
    await flushPromises();

    // Unmount fires onBeforeUnmount → nulls pendingBody (the unmount half of F1).
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
      content: null,
      op: "edit",
      choice: "keep_mine",
      expectedRemoteOid: PAYLOAD.remote_tip,
      name: PAYLOAD.name,
    });
  });
});
