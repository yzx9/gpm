// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import CloneFlow from "./CloneFlow.vue";

vi.mock("@tauri-apps/api/core");

// Capture the Android-back handler so a test can fire a hardware back press and
// assert it collapses step 2 → 1 (instead of popping the Setup route). Mirrors
// the deferred-listener pattern in useOverlayBackHandler.test.ts.
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

describe("CloneFlow Android-back intercept", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("repo ready → arms the back listener on step 2; firing it collapses to step 1", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "is_repo_ready") return Promise.resolve(true);
      if (cmd === "list_recipients") return Promise.resolve([]);
      return Promise.resolve(undefined);
    });

    mountWithApp(CloneFlow);
    await flushPromises();
    // Auto-advanced to step 2 → listener armed.
    expect(api.onBackButtonPress).toHaveBeenCalledTimes(1);

    // Hardware back on step 2 collapses to step 1 (listener disarms).
    api.fireBack();
    await flushPromises();
    expect(api.unregister).toHaveBeenCalled();
  });

  it("repo not ready → stays on step 1, no back listener armed", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "is_repo_ready") return Promise.resolve(false);
      return Promise.resolve(undefined);
    });

    mountWithApp(CloneFlow);
    await flushPromises();
    expect(api.onBackButtonPress).not.toHaveBeenCalled();
  });
});
