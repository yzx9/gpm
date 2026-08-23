// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import type { Router } from "vue-router";

import type { AppLockStore } from "./useAppLockState";
import { createForegroundSyncStore } from "./useForegroundSync";

/**
 * Drive the composable with a controllable fake app-lock store + router. The
 * composable only reads `.value` off the app-lock refs, so plain refs suffice.
 */
function fakeAppLockStore(
  over: { enabled?: boolean; locked?: boolean; ready?: boolean } = {},
): AppLockStore {
  return {
    appLockEnabled: ref(over.enabled ?? false),
    appLocked: ref(over.locked ?? false),
    appReady: ref(over.ready ?? true),
    init: vi.fn(),
    setUnlockInFlight: vi.fn(),
    dispose: vi.fn(),
  } as unknown as AppLockStore;
}

function fakeRouter(): Router {
  return { push: vi.fn().mockResolvedValue(undefined) } as unknown as Router;
}

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

/** A fast-forwarded outcome (changed) — the "silent refresh" path. */
const FF_CHANGED = {
  kind: "fast_forwarded",
  changed: true,
  head: "abc1234",
  authenticity: {
    mode: "off",
    new_commits: [],
    open_issues: [],
    blocked: false,
  },
} as const;

/** A diverged outcome — sets the passive attention badge. */
const DIVERGED = {
  kind: "diverged",
  local_ahead: 1,
  remote_ahead: 1,
  remote_tip: "deadbeef",
  local_only_entries: [],
  modified_entries: [],
  other_changed_files: [],
} as const;

/**
 * `invoke` dispatch mock (NOT a `mockResolvedValueOnce` queue — see the
 * `[vitest-clearAllmocks-keeps-impl-once-queue-drift]` learning): dispatches by
 * command name so extra calls don't drift sibling state.
 */
function mockInvoke(cfg: { autosync: boolean; fg: unknown }) {
  vi.mocked(invoke).mockImplementation(async (cmd: string) => {
    if (cmd === "get_app_config")
      return {
        autosync: cfg.autosync,
        repositories: ["test-repo"],
        last_active: "test-repo",
      };
    if (cmd === "background_sync") return cfg.fg;
    return undefined;
  });
}

/** Flush the composable's async `maybeSync` microtasks. */
const flush = () => new Promise((r) => setTimeout(r, 0));

describe("useForegroundSync", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("cold-start syncs when AutoSync is on, ready, and unlocked (silent on FF)", async () => {
    mockInvoke({ autosync: true, fg: FF_CHANGED });
    const fg = createForegroundSyncStore(
      fakeAppLockStore({ ready: true }),
      fakeRouter(),
    );

    await flush();

    expect(invoke).toHaveBeenCalledWith("background_sync", {
      repoId: "test-repo",
    });
    expect(fg.syncAttention.value).toBeNull(); // success is silent
  });

  it("does NOT sync when AutoSync is off", async () => {
    mockInvoke({ autosync: false, fg: FF_CHANGED });
    createForegroundSyncStore(fakeAppLockStore({ ready: true }), fakeRouter());

    await flush();

    expect(invoke).not.toHaveBeenCalledWith("background_sync");
  });

  it("does NOT sync while app-locked", async () => {
    mockInvoke({ autosync: true, fg: FF_CHANGED });
    createForegroundSyncStore(
      fakeAppLockStore({ ready: true, locked: true }),
      fakeRouter(),
    );

    await flush();

    expect(invoke).not.toHaveBeenCalledWith("background_sync");
  });

  it("does NOT sync before the app-lock store is ready", async () => {
    mockInvoke({ autosync: true, fg: FF_CHANGED });
    createForegroundSyncStore(fakeAppLockStore({ ready: false }), fakeRouter());

    await flush();

    expect(invoke).not.toHaveBeenCalledWith("background_sync");
  });

  it("sets the attention badge on a diverged outcome (never a modal)", async () => {
    mockInvoke({ autosync: true, fg: DIVERGED });
    const fg = createForegroundSyncStore(
      fakeAppLockStore({ ready: true }),
      fakeRouter(),
    );

    await flush();

    expect(invoke).toHaveBeenCalledWith("background_sync", {
      repoId: "test-repo",
    });
    expect(fg.syncAttention.value).toEqual(DIVERGED);
  });

  it("engage() routes to the list and retains the badge (not cleared on tap)", async () => {
    mockInvoke({ autosync: true, fg: DIVERGED });
    const router = fakeRouter();
    const fg = createForegroundSyncStore(
      fakeAppLockStore({ ready: true }),
      router,
    );

    await flush();
    expect(fg.syncAttention.value).toEqual(DIVERGED);

    fg.engage();

    // retained until a later clean sync reconciles — clearing on tap would
    // let an unresolved divergence go silent while foregrounded.
    expect(fg.syncAttention.value).toEqual(DIVERGED);
    expect(router.push).toHaveBeenCalledWith({ name: "entries" });
  });

  it("a skipped/failed sync (null) is silent and does not set the badge", async () => {
    mockInvoke({ autosync: true, fg: null });
    const fg = createForegroundSyncStore(
      fakeAppLockStore({ ready: true }),
      fakeRouter(),
    );

    await flush();

    expect(invoke).toHaveBeenCalledWith("background_sync", {
      repoId: "test-repo",
    });
    expect(fg.syncAttention.value).toBeNull();
  });

  it("resume (app-resumed) syncs when the app-lock gate is off", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-01-01T00:00:00Z"));
    mockInvoke({ autosync: true, fg: FF_CHANGED });
    // Start not-ready so the cold-start watch no-ops; the appReady flip below is
    // the cold-start sync, and the resume must independently fire a SECOND one.
    const store = fakeAppLockStore({ ready: false, enabled: false });
    const fg = createForegroundSyncStore(store, fakeRouter());
    fg.init();
    (store.appReady as unknown as { value: boolean }).value = true;
    await vi.runAllTimersAsync();
    const coldCount = vi
      .mocked(invoke)
      .mock.calls.filter((c) => c[0] === "background_sync").length;
    expect(coldCount).toBe(1);

    // Fast-forward past the 60s throttle, then a resume must sync again — this
    // asserts the resume path fires, not just the cold-start path.
    vi.setSystemTime(new Date("2026-01-01T00:01:30Z"));
    fireResume();
    await vi.runAllTimersAsync();
    const resumeCount = vi
      .mocked(invoke)
      .mock.calls.filter((c) => c[0] === "background_sync").length;
    expect(resumeCount).toBe(2);
    expect(fg.syncAttention.value).toBeNull();

    fg.dispose();
  });

  it("does not double-sync within the 60s throttle", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-01-01T00:00:00Z"));
    mockInvoke({ autosync: true, fg: FF_CHANGED });
    const store = fakeAppLockStore({ ready: false, enabled: false });
    const fg = createForegroundSyncStore(store, fakeRouter());
    fg.init();
    (store.appReady as unknown as { value: boolean }).value = true;
    await vi.runAllTimersAsync();
    const firstCount = vi
      .mocked(invoke)
      .mock.calls.filter((c) => c[0] === "background_sync").length;
    expect(firstCount).toBe(1);

    // A resume 10s later is throttled.
    vi.setSystemTime(new Date("2026-01-01T00:00:10Z"));
    fireResume();
    await vi.runAllTimersAsync();

    const secondCount = vi
      .mocked(invoke)
      .mock.calls.filter((c) => c[0] === "background_sync").length;
    expect(secondCount).toBe(1); // still one — throttled

    fg.dispose();
  });

  it("sets the badge on an Enforce-block fast-forward (HEAD did not advance)", async () => {
    const BLOCKED_FF = {
      kind: "fast_forwarded",
      changed: false,
      head: "oldtip",
      authenticity: {
        mode: "enforce",
        new_commits: [],
        open_issues: [{ hash: "x", subject: "s" }],
        blocked: true,
      },
    } as const;
    mockInvoke({ autosync: true, fg: BLOCKED_FF });
    const fg = createForegroundSyncStore(
      fakeAppLockStore({ ready: true }),
      fakeRouter(),
    );
    await flush();
    expect(fg.syncAttention.value).toEqual(BLOCKED_FF);
  });

  it("syncs after biometric unlock when cold-started under app-lock", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-01-01T00:00:00Z"));
    mockInvoke({ autosync: true, fg: FF_CHANGED });
    const store = fakeAppLockStore({
      enabled: true,
      locked: true,
      ready: true,
    });
    const fg = createForegroundSyncStore(store, fakeRouter());
    await vi.runAllTimersAsync();
    expect(invoke).not.toHaveBeenCalledWith("background_sync"); // locked ⇒ skip
    (store.appLocked as unknown as { value: boolean }).value = false; // unlock
    await vi.runAllTimersAsync();
    expect(invoke).toHaveBeenCalledWith("background_sync", {
      repoId: "test-repo",
    });
    fg.dispose();
  });

  it("resume-syncs on a grace return (gate enabled + unlocked) — R058", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-01-01T00:00:00Z"));
    mockInvoke({ autosync: true, fg: FF_CHANGED });
    const store = fakeAppLockStore({
      enabled: true,
      ready: true,
      locked: false,
    });
    const fg = createForegroundSyncStore(store, fakeRouter());
    fg.init();
    await vi.runAllTimersAsync();
    expect(
      vi.mocked(invoke).mock.calls.filter((c) => c[0] === "background_sync")
        .length,
    ).toBe(1); // cold-start ran
    // Past the throttle, a grace-window resume (gate on + unlocked) MUST sync —
    // the old `appLockEnabled` bail is gone (R058: it was for every-resume re-lock;
    // under grace the app stays unlocked, so skipping the sync would lose updates).
    vi.setSystemTime(new Date("2026-01-01T00:02:00Z"));
    fireResume();
    await vi.runAllTimersAsync();
    expect(
      vi.mocked(invoke).mock.calls.filter((c) => c[0] === "background_sync")
        .length,
    ).toBe(2);
    fg.dispose();
  });

  it("does not sync when getAppConfig rejects (can't read config)", async () => {
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === "get_app_config") throw new Error("config unreadable");
      return undefined;
    });
    createForegroundSyncStore(fakeAppLockStore({ ready: true }), fakeRouter());
    await flush();
    expect(invoke).toHaveBeenCalledWith("get_app_config");
    expect(invoke).not.toHaveBeenCalledWith("background_sync");
  });

  it("treats a foregroundSync() rejection as a silent skip that is not throttled", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-01-01T00:00:00Z"));
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === "get_app_config")
        return {
          autosync: true,
          repositories: ["test-repo"],
          last_active: "test-repo",
        };
      if (cmd === "background_sync") throw new Error("net down");
      return undefined;
    });
    const fg = createForegroundSyncStore(
      fakeAppLockStore({ ready: true }),
      fakeRouter(),
    );
    fg.init();
    await vi.runAllTimersAsync();
    expect(fg.syncAttention.value).toBeNull(); // silent
    // lastForegroundSyncAt not updated on failure ⇒ a near-immediate resume retries.
    vi.setSystemTime(new Date("2026-01-01T00:00:00.100Z"));
    fireResume();
    await vi.runAllTimersAsync();
    expect(
      vi.mocked(invoke).mock.calls.filter((c) => c[0] === "background_sync")
        .length,
    ).toBe(2);
    fg.dispose();
  });

  it("clears a prior divergence badge when a later sync reconciles cleanly", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-01-01T00:00:00Z"));
    let call = 0;
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === "get_app_config")
        return {
          autosync: true,
          repositories: ["test-repo"],
          last_active: "test-repo",
        };
      if (cmd === "background_sync")
        return call++ === 0 ? DIVERGED : FF_CHANGED;
      return undefined;
    });
    const fg = createForegroundSyncStore(
      fakeAppLockStore({ ready: true }),
      fakeRouter(),
    );
    fg.init();
    await vi.runAllTimersAsync();
    expect(fg.syncAttention.value).toEqual(DIVERGED);
    vi.setSystemTime(new Date("2026-01-01T00:02:00Z"));
    fireResume();
    await vi.runAllTimersAsync();
    expect(fg.syncAttention.value).toBeNull(); // clean FF cleared it
    fg.dispose();
  });
});
