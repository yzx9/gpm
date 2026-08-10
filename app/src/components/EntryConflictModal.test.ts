// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import EntryConflictModal from "@/components/EntryConflictModal.vue";
import type { EntryConflictPayload } from "@/composables/useEntryConflict";
import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
import {
  enableAutoUnmount,
  flushPromises,
  type ComponentMountingOptions,
} from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// Mirror DivergenceModal.test.ts: mount the modal inside the app-shell provide
// block (it injects useSecretReveal → useSecuritySettings + useSecureScreen +
// useLockState, plus the BaseModalShell needs useScrollLock + useOverlayBackHandler).
// enableAutoUnmount drains the scroll-lock + back-handler between tests so the
// counts don't climb across the two BaseModalShells each mount renders.
enableAutoUnmount(afterEach);

const CONFLICT: EntryConflictPayload = {
  name: "servers/prod",
  base_oid: "base-aaa",
  current_oid: "curr-bbb",
  remote_tip: "tip-ccc",
  op: "edit",
};

const SHEET = '[aria-label="This secret changed elsewhere"]';
// Step-2 aria-label depends on which choice opened it.
const STEP2_KEEPTHEIRS = '[aria-label="Keep their version"]';

function mountModal(
  options: ComponentMountingOptions<typeof EntryConflictModal>,
) {
  // mountWithApp forwards mountOpts under its 8-key provide block; we keep it
  // for parity with the page tests even though bare `mount` would also work
  // given the global setup.ts i18n + per-test provide.
  const { wrapper } = mountWithApp(EntryConflictModal, { mountOpts: options });
  return wrapper;
}

describe("EntryConflictModal", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders the op-keyed heading + entry name for an edit conflict", async () => {
    const wrapper = mountModal({ props: { conflict: CONFLICT } });
    await flushPromises();

    expect(wrapper.find(SHEET).exists()).toBe(true);
    // Edit op heading (headingEdit) — distinct from delete/create.
    expect(wrapper.text()).toContain("This secret changed elsewhere");
    // The entry name renders inside the danger divider block.
    expect(wrapper.find(".ec-name code").text()).toBe("servers/prod");
  });

  it("clicking keep-theirs opens step-2 confirm; confirm emits 'resolve' with 'keep_theirs'", async () => {
    const wrapper = mountModal({ props: { conflict: CONFLICT } });
    await flushPromises();

    // Step 1 outline button = "Use their version" (useTheirsEdit).
    const keepTheirs = wrapper
      .findAll("button")
      .find((b) => b.text().includes("Use their version"))!;
    await keepTheirs.trigger("click");
    await flushPromises();

    // Step-2 confirm is up.
    expect(wrapper.find(STEP2_KEEPTHEIRS).exists()).toBe(true);

    // The step-2 confirm button (variant=danger) emits "resolve" with the
    // pending choice; it does NOT call any IPC directly.
    const confirmBtn = wrapper
      .findAll("button")
      .find((b) => b.text().includes("Discard my edit"))!;
    await confirmBtn.trigger("click");
    await flushPromises();

    expect(wrapper.emitted("resolve")).toEqual([["keep_theirs"]]);
  });

  it("'Preview their version' calls show_password(entryPath=name) and reveals the value in the panel", async () => {
    // withClaim acquires FLAG_SECURE first (plugin:screen-secure|set_secure),
    // then runWithAuth runs show_password — identity is cached by default in
    // mountWithApp, so no auth overlay. Dispatch by command name to stay
    // order-agnostic (matches the copyPassword test's caution about queues).
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "plugin:screen-secure|set_secure") {
        return Promise.resolve(undefined);
      }
      if (cmd === "show_password") {
        return Promise.resolve({
          password: "teammate-pw",
          notes: "their notes",
          has_totp: false,
          version: "oid-x",
        });
      }
      return Promise.resolve(undefined);
    });
    const wrapper = mountModal({ props: { conflict: CONFLICT } });
    await flushPromises();

    // No preview yet — the reveal panel is gated on `revealed`.
    expect(wrapper.find(".ec-secret").exists()).toBe(false);

    const previewBtn = wrapper.find(".ec-preview-btn");
    expect(previewBtn.exists()).toBe(true);
    await previewBtn.trigger("click");
    await flushPromises();

    // show_password was called with the entry name (the teammate's current
    // version IS the local HEAD at the conflict moment).
    expect(invoke).toHaveBeenCalledWith("show_password", {
      repoId: "test-repo",
      entryPath: "servers/prod",
    });
    // The teammate's password is now visible in the preview panel.
    expect(wrapper.find(".ec-secret code").text()).toBe("teammate-pw");
    expect(wrapper.text()).toContain("teammate-pw");
  });

  it("setting :error returns the modal to the selection sheet (pendingChoice reset)", async () => {
    const wrapper = mountModal({ props: { conflict: CONFLICT } });
    await flushPromises();

    // Open step 2 (keep-theirs confirm).
    const keepTheirs = wrapper
      .findAll("button")
      .find((b) => b.text().includes("Use their version"))!;
    await keepTheirs.trigger("click");
    await flushPromises();
    expect(wrapper.find(STEP2_KEEPTHEIRS).exists()).toBe(true);

    // Parent sets :error after a failed resolve — the watch resets
    // pendingChoice, dropping step 2 back to the sheet.
    await wrapper.setProps({ error: "Could not resolve the divergence" });
    await flushPromises();

    expect(wrapper.find(STEP2_KEEPTHEIRS).exists()).toBe(false);
    expect(wrapper.find(SHEET).exists()).toBe(true);
    // The error line itself renders on the sheet.
    expect(wrapper.text()).toContain("Could not resolve the divergence");
  });

  it("setting :conflict to null wipes any previewed secret (password no longer in DOM)", async () => {
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "plugin:screen-secure|set_secure") {
        return Promise.resolve(undefined);
      }
      if (cmd === "show_password") {
        return Promise.resolve({
          password: "teammate-pw",
          notes: "",
          has_totp: false,
          version: null,
        });
      }
      return Promise.resolve(undefined);
    });
    const wrapper = mountModal({ props: { conflict: CONFLICT } });
    await flushPromises();

    // Reveal their version first.
    await wrapper.find(".ec-preview-btn").trigger("click");
    await flushPromises();
    expect(wrapper.text()).toContain("teammate-pw");

    // Parent closes the modal (conflict → null). The watch fires clear(),
    // wiping the previewed plaintext so it doesn't outlive the modal.
    await wrapper.setProps({ conflict: null });
    await flushPromises();

    expect(wrapper.text()).not.toContain("teammate-pw");
    // The sheet itself is gone too (v-if="conflict").
    expect(wrapper.find(SHEET).exists()).toBe(false);
  });

  it("renders the create-specific heading + buttons for a create conflict", async () => {
    const wrapper = mountModal({
      props: { conflict: { ...CONFLICT, op: "create" } },
    });
    await flushPromises();

    expect(wrapper.find(SHEET).exists()).toBe(true);
    // headingCreate — distinct from the edit/delete headings.
    expect(wrapper.text()).toContain("This name is already in use");
    const buttons = wrapper.findAll("button");
    // Step-1 labels are op-keyed: keepTheirsCreate + overwriteCreate.
    expect(
      buttons.some((b) => b.text().includes("Keep the existing one")),
    ).toBe(true);
    expect(buttons.some((b) => b.text().includes("Overwrite with mine"))).toBe(
      true,
    );
  });

  it("renders the delete-specific heading + buttons for a delete conflict", async () => {
    const wrapper = mountModal({
      props: { conflict: { ...CONFLICT, op: "delete" } },
    });
    await flushPromises();

    expect(wrapper.find(SHEET).exists()).toBe(true);
    // headingDelete — distinct from the edit/create headings.
    expect(wrapper.text()).toContain("This secret was changed elsewhere");
    const buttons = wrapper.findAll("button");
    // Step-1 labels: keepTheirsDelete + deleteAnyway.
    expect(buttons.some((b) => b.text().includes("Keep their version"))).toBe(
      true,
    );
    expect(buttons.some((b) => b.text().includes("Delete it anyway"))).toBe(
      true,
    );
  });

  it("'Preview their version' surfaces previewError when show_password rejects (undecryptable)", async () => {
    // Recipient rotation can make the teammate's blob undecryptable for the local
    // identity — the preview must surface a clear error, not crash the modal, and
    // keep-theirs stays a stated leap of faith.
    vi.mocked(invoke).mockImplementation((cmd: string) => {
      if (cmd === "plugin:screen-secure|set_secure")
        return Promise.resolve(undefined);
      if (cmd === "show_password")
        return Promise.reject({
          code: "DECRYPT_FAILED",
          message: "can't decrypt their version",
        });
      return Promise.resolve(undefined);
    });
    const wrapper = mountModal({ props: { conflict: CONFLICT } });
    await flushPromises();

    await wrapper.find(".ec-preview-btn").trigger("click");
    await flushPromises();

    // No reveal panel — the secret never decrypted.
    expect(wrapper.find(".ec-secret").exists()).toBe(false);
    // The error line renders.
    expect(wrapper.text()).toContain("can't decrypt their version");
    // The preview button is gone — the v-else-if="previewError" <p> replaces it,
    // so keep-theirs stays a stated leap of faith (or the user cancels).
    expect(wrapper.find(".ec-preview-btn").exists()).toBe(false);
  });

  it("resolving=true spins the confirm button (step 2) and shows the busy label", async () => {
    const wrapper = mountModal({
      props: { conflict: CONFLICT, resolving: true },
    });
    await flushPromises();

    // Open step 2 (keep-theirs) — the confirm then reflects the resolving state.
    const keepTheirs = wrapper
      .findAll("button")
      .find((b) => b.text().includes("Use their version"))!;
    await keepTheirs.trigger("click");
    await flushPromises();

    // BaseButton :loading renders a spinner + the busy label (keep_theirs → Discarding…).
    expect(wrapper.findComponent({ name: "BaseSpinner" }).exists()).toBe(true);
    expect(wrapper.text()).toContain("Discarding");
  });

  it("step-2 Cancel returns to the selection sheet without resolving or closing", async () => {
    const wrapper = mountModal({ props: { conflict: CONFLICT } });
    await flushPromises();

    const keepTheirs = wrapper
      .findAll("button")
      .find((b) => b.text().includes("Use their version"))!;
    await keepTheirs.trigger("click");
    await flushPromises();
    expect(wrapper.find(STEP2_KEEPTHEIRS).exists()).toBe(true);

    // The step-2 Cancel is the last "Cancel" button (the sheet's cancel renders first).
    const cancels = wrapper
      .findAll("button")
      .filter((b) => b.text().includes("Cancel"));
    await cancels[cancels.length - 1].trigger("click");
    await flushPromises();

    // Retreated to the sheet — step 2 gone, sheet still up, no resolve/close emitted.
    expect(wrapper.find(STEP2_KEEPTHEIRS).exists()).toBe(false);
    expect(wrapper.find(SHEET).exists()).toBe(true);
    expect(wrapper.emitted("resolve")).toBeFalsy();
    expect(wrapper.emitted("close")).toBeFalsy();
  });
});
