// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import type { SyncDivergence } from "@/api";
import { enableAutoUnmount, flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import DivergenceModal from "./DivergenceModal.vue";

// DivergenceModal mounts BaseModalShell(s), which lock the document scroller on
// mount (useScrollLock). Unmount every wrapper after each test so the shared
// lock count returns to 0 instead of climbing across tests that mount without an
// explicit unmount.
enableAutoUnmount(afterEach);

// Deferred-mock (mirrors BaseModalShell.test.ts / useOverlayBackHandler.test.ts)
// so tests can drive "back pressed". Each BaseModalShell inside DivergenceModal
// registers its own listener; Tauri fans one back press to ALL registered
// listeners, so fireBack() delivers to every currently-mounted shell.
const api = vi.hoisted(() => {
  let handler: ((p: { canGoBack: boolean }) => void) | null = null;
  const unregister = vi.fn(async () => {
    handler = null;
  });
  const onBackButtonPress = vi.fn((h: (p: { canGoBack: boolean }) => void) => {
    handler = h;
    return Promise.resolve({ unregister });
  });
  const fireBack = () => {
    handler?.({ canGoBack: false });
  };
  return { onBackButtonPress, unregister, fireBack };
});
vi.mock("@tauri-apps/api/app", () => ({
  onBackButtonPress: api.onBackButtonPress,
}));

const DIV: SyncDivergence = {
  local_ahead: 1,
  remote_ahead: 0,
  remote_tip: "abc123abc123abc123abc123abc123abc123abc1",
  local_only_entries: ["secret/foo"],
  modified_entries: [],
  other_changed_files: [],
};

const STEP1 = '[aria-label="Local and remote have diverged"]';
// Step-2 aria-label depends on which choice opened it; adopt_remote is the
// raw `<button class="btn-danger">`, so opening via that yields the discard label.
const STEP2_ADOPT = '[aria-label="Discard your local commit"]';

describe("DivergenceModal back/backdrop coordination", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("step 1: back emits `close` (cancelAll) to the parent", async () => {
    const wrapper = mount(DivergenceModal, {
      props: { divergence: DIV, context: "sync" },
    });
    await flushPromises();
    expect(wrapper.find(STEP1).exists()).toBe(true);

    api.fireBack();
    await flushPromises();

    expect(wrapper.emitted("close")).toHaveLength(1);
  });

  it("step 2: back returns to step 1 — no parent `close`, step 2 dismissed", async () => {
    const wrapper = mount(DivergenceModal, {
      props: { divergence: DIV, context: "sync" },
    });
    await flushPromises();
    // Open step 2 (adopt-remote confirm) via the Adopt button.
    await wrapper.find("button.btn-danger").trigger("click");
    await flushPromises();
    expect(wrapper.find(STEP2_ADOPT).exists()).toBe(true);

    api.fireBack();
    await flushPromises();

    // cancelConfirm is internal (drops pendingChoice); the parent sees no close.
    expect(wrapper.emitted("close")).toBeUndefined();
    // Step 2 gone, step 1 still up.
    expect(wrapper.find(STEP2_ADOPT).exists()).toBe(false);
    expect(wrapper.find(STEP1).exists()).toBe(true);
  });

  it("step 2 while resolving: back AND backdrop are both trapped (no dismiss, no close)", async () => {
    const wrapper = mount(DivergenceModal, {
      props: { divergence: DIV, context: "sync", resolving: true },
    });
    await flushPromises();
    await wrapper.find("button.btn-danger").trigger("click");
    await flushPromises();
    const step2 = wrapper.find(STEP2_ADOPT);
    expect(step2.exists()).toBe(true);

    // Back press: step 1 swallows (!pendingChoice=false), step 2 swallows
    // (!resolving=false). Net: fully trapped.
    api.fireBack();
    await flushPromises();
    expect(wrapper.emitted("close")).toBeUndefined();
    expect(wrapper.find(STEP2_ADOPT).exists()).toBe(true);

    // Backdrop tap on step 2's overlay: dismissOnBackdrop=!resolving=false → trapped.
    await step2.trigger("click");
    await flushPromises();
    expect(wrapper.emitted("close")).toBeUndefined();
    expect(wrapper.find(STEP2_ADOPT).exists()).toBe(true);
  });
});
