// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import {
  APP_LOCK_KEY,
  BACK_HANDLER_KEY,
  createAppLockStore,
  createBackHandlerRegistry,
  createDialog,
  createLockState,
  createScrollLockController,
  createSecureScreen,
  createSecuritySettings,
  createToast,
  DIALOG_KEY,
  LOCK_KEY,
  SCROLL_LOCK_KEY,
  SECURE_SCREEN_KEY,
  SECURITY_SETTINGS_KEY,
  TOAST_KEY,
} from "@/composables";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { defineComponent } from "vue";
import AppLockOverlay from "./AppLockOverlay.vue";
import DialogHost from "./DialogHost.vue";

vi.mock("@tauri-apps/api/core");
// Stub the cold-start locale reconcile so it doesn't fire an extra invoke that
// would consume the test's sequenced mocks (same pattern as AppLockOverlay.test).
vi.mock("@/i18n", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/i18n")>();
  return {
    ...actual,
    reconcileLocaleFromBackend: vi.fn().mockResolvedValue(undefined),
  };
});
vi.mock("@/i18n/native", () => ({
  appLockUnlockPrompt: vi.fn(() => ({
    title: "t",
    subtitle: "s",
    negative: "n",
  })),
}));

// A faithful slice of App.vue's .app-shell: the gate overlay AND the dialog host
// as siblings, with DialogHost rendered AFTER the gate — the load-bearing order
// pinned by the comment in App.vue. Both share ONE dialog instance so a real
// confirm flows from the lock screen's diagnostics link through to DialogHost's
// rendered overlay.
function mountShell() {
  const dialog = createDialog(); // real, un-spied — confirm reaches the host
  const Shell = defineComponent({
    components: { AppLockOverlay, DialogHost },
    template: `<div class="app-shell"><AppLockOverlay /><DialogHost /></div>`,
  });
  const wrapper = mount(Shell, {
    global: {
      provide: {
        [LOCK_KEY]: createLockState({ unlocked: true }),
        [APP_LOCK_KEY]: createAppLockStore(),
        [SECURE_SCREEN_KEY]: createSecureScreen({ available: true }),
        [SECURITY_SETTINGS_KEY]: createSecuritySettings(),
        [TOAST_KEY]: createToast(),
        [DIALOG_KEY]: dialog,
        [SCROLL_LOCK_KEY]: createScrollLockController(),
        [BACK_HANDLER_KEY]: createBackHandlerRegistry(),
      },
    },
  });
  return { wrapper, dialog };
}

describe("AppLockOverlay + DialogHost stacking", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(invoke).mockResolvedValue(undefined);
  });

  it("a gate-fired confirm renders AFTER the gate, so equal-z tree order puts it on top", async () => {
    // Regression guard for the in-lock diagnostics confirm (P1): both the gate
    // and the gate-fired confirm resolve to z-index Z.gate, so paint order is
    // DOM/tree order (CSS2 §E). The confirm MUST follow the gate, else the
    // opaque gate hides it. App.vue keeps DialogHost last for this reason.
    const { wrapper } = mountShell();
    await flushPromises(); // mount + any auto-prompt settle

    const gate = wrapper.find(".overlay.fullscreen");
    expect(gate.exists()).toBe(true);

    // Tap the diagnostics link → real confirm → DialogHost renders it.
    const link = wrapper
      .findAll("button")
      .find((b) => b.text().includes("Export diagnostics"))!;
    expect(link).toBeTruthy();
    await link.trigger("click");
    await flushPromises();

    const confirm = wrapper.find(".overlay:not(.fullscreen)");
    expect(confirm.exists()).toBe(true);

    // Same z-index (both Z.gate) → the later-DOM element wins. Confirm must
    // FOLLOW the gate; if it precedes it, the gate paints over the confirm.
    const rel = gate.element.compareDocumentPosition(confirm.element);
    expect(rel & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });
});
