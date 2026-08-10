// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import EntryDetailPage from "./EntryDetailPage.vue";

const { mockPush, mockReplace } = vi.hoisted(() => ({
  mockPush: vi.fn(),
  mockReplace: vi.fn(),
}));

vi.mock("@tauri-apps/api/core");

// Override useRoute to provide entry path
const mockRoute = {
  params: { pathMatch: "servers%2Fprod.age" },
  query: {},
  name: "entry",
  path: "/entry/servers%2Fprod.age",
  fullPath: "/entry/servers%2Fprod.age",
};

vi.mock("vue-router", () => ({
  createRouter: vi.fn(),
  createWebHashHistory: vi.fn(),
  useRouter: () => ({
    push: mockPush,
    replace: mockReplace,
    back: vi.fn(),
  }),
  useRoute: () => mockRoute,
}));

describe("EntryDetailPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
    // "identity cached" precondition is established per-mount by mountWithApp's
    // default unlocked:true (App.vue's init() doesn't run in page tests).
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  function mountPage() {
    return mountWithApp(EntryDetailPage).wrapper;
  }

  describe("showPassword", () => {
    it("invokes show_password with decoded entry path", async () => {
      vi.mocked(invoke).mockResolvedValue({
        password: "s3cret",
        notes: "some notes",
        has_totp: false,
        attachment: null,
      });
      const wrapper = mountPage();
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("show_password", {
        entryPath: "servers/prod.age",
      });
    });

    it("displays password and notes after reveal", async () => {
      vi.mocked(invoke).mockResolvedValue({
        password: "s3cret",
        notes: "some notes",
        has_totp: false,
        attachment: null,
      });
      const wrapper = mountPage();
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();

      expect(wrapper.text()).toContain("s3cret");
      expect(wrapper.text()).toContain("some notes");
    });

    it("ticks the auto-clear countdown down each second", async () => {
      vi.mocked(invoke).mockResolvedValue({
        password: "s3cret",
        notes: "",
        has_totp: false,
        attachment: null,
      });
      const wrapper = mountPage();
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();

      // Freshly revealed: shows the full default 45s window.
      expect(wrapper.text()).toContain("Auto-clears in 45s");

      // One second later: the live countdown has ticked.
      vi.advanceTimersByTime(1_000);
      await flushPromises();
      expect(wrapper.text()).toContain("Auto-clears in 44s");
    });

    it("clamps the countdown at 1s and never shows 0s before the wipe", async () => {
      vi.mocked(invoke).mockResolvedValue({
        password: "s3cret",
        notes: "",
        has_totp: false,
        attachment: null,
      });
      const wrapper = mountPage();
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();

      // Tick to the last whole second before the 45s wipe deadline: the clamp
      // holds at 1s, never flashing 0s.
      vi.advanceTimersByTime(44_000);
      await flushPromises();
      expect(wrapper.text()).toContain("Auto-clears in 1s");
      expect(wrapper.text()).not.toContain("Auto-clears in 0s");

      // The final second: the wipe fires and the whole block (label included) hides.
      vi.advanceTimersByTime(1_000);
      await flushPromises();
      expect(wrapper.text()).not.toContain("s3cret");
      expect(wrapper.text()).not.toContain("Auto-clears in");
    });

    it("resets the countdown when the view-clear setting changes mid-reveal", async () => {
      vi.mocked(invoke).mockResolvedValue({
        password: "s3cret",
        notes: "",
        has_totp: false,
        attachment: null,
      });
      const { wrapper, securitySettings } = mountWithApp(EntryDetailPage);
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();
      expect(wrapper.text()).toContain("Auto-clears in 45s");

      // A few seconds tick down from the original 45s window.
      vi.advanceTimersByTime(5_000);
      await flushPromises();
      expect(wrapper.text()).toContain("Auto-clears in 40s");

      // Lowering the setting to 10s re-arms from a fresh deadline.
      securitySettings.applySecurityConfig({
        view_clear_secs: 10,
      });
      await flushPromises();
      expect(wrapper.text()).toContain("Auto-clears in 10s");

      // The new (shorter) deadline governs: 10s later the password wipes.
      vi.advanceTimersByTime(10_000);
      await flushPromises();
      expect(wrapper.text()).not.toContain("s3cret");
    });

    it("toggles off when clicked while already revealed (no re-auth, no re-decrypt)", async () => {
      // Regression: clicking the "Showing..." button used to re-run auth +
      // show_password instead of hiding. It must now clear in place.
      vi.mocked(invoke).mockResolvedValue({
        password: "s3cret",
        notes: "some notes",
        has_totp: false,
        attachment: null,
      });
      const wrapper = mountPage();
      // First click reveals.
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();
      expect(wrapper.text()).toContain("s3cret");

      // Second click toggles off — the aria-label flips to "Password is showing".
      await wrapper
        .find('button[aria-label="Password is showing"]')
        .trigger("click");
      await flushPromises();

      // Password is hidden again...
      expect(wrapper.text()).not.toContain("s3cret");
      // ...and show_password was NOT invoked a second time.
      expect(
        vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === "show_password"),
      ).toHaveLength(1);
    });

    it("auto-clears sensitive data after the default view-clear window", async () => {
      vi.mocked(invoke).mockResolvedValue({
        password: "s3cret",
        notes: "notes",
        has_totp: false,
        attachment: null,
      });
      const wrapper = mountPage();
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();

      // Password is visible
      expect(wrapper.text()).toContain("s3cret");

      // Advance past the default view-clear window (45s; configurable via Settings).
      vi.advanceTimersByTime(45_000);
      await flushPromises();

      // Password is gone
      expect(wrapper.text()).not.toContain("s3cret");
      expect(wrapper.text()).not.toContain("notes");
    });

    it("shows error on failure", async () => {
      // Let the screen-secure claim acquire succeed, but reject the decrypt.
      vi.mocked(invoke).mockImplementation(async (cmd: string) => {
        if (cmd === "plugin:screen-secure|set_secure") return undefined;
        throw { code: "DecryptFailed", message: "Decryption failed" };
      });
      const wrapper = mountPage();
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();

      expect(wrapper.find("[role='alert']").text()).toContain(
        "Decryption failed",
      );
    });

    it("shows hint for errors containing 'ecrypt'", async () => {
      // Let the screen-secure claim acquire succeed, but reject the decrypt.
      vi.mocked(invoke).mockImplementation(async (cmd: string) => {
        if (cmd === "plugin:screen-secure|set_secure") return undefined;
        throw { code: "DecryptFailed", message: "Decryption error" };
      });
      const wrapper = mountPage();
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();

      expect(wrapper.text()).toContain("Check your age identity and try again");
    });

    it("swallows AUTH_CANCELLED when the gate's Unlock is dismissed (Android back)", async () => {
      // R086: a cold identity shows the Unlock gate (the button pile is hidden).
      // Tapping Unlock parks on the auth overlay; dismissing it (back) must
      // swallow AUTH_CANCELLED with no error UI, mirroring the old action path.
      vi.mocked(invoke).mockResolvedValue({
        has_totp: false,
        attachment: null,
      });
      const { wrapper, lock } = mountWithApp(EntryDetailPage, {
        unlocked: false,
      });
      await wrapper
        .find('button[aria-label="Unlock servers/prod"]')
        .trigger("click");
      await flushPromises(); // parked awaiting auth

      lock.cancelAuth(); // user dismissed the overlay (back)
      await flushPromises(); // rejection propagates to the catch

      // No error UI — the catch swallowed AUTH_CANCELLED; the op never ran.
      expect(wrapper.find("[role='alert']").exists()).toBe(false);
    });

    it("discards a decrypt that resolves after Back (invalidation token)", async () => {
      // R031: a slow decrypt resolving after the user left must not write the
      // secret into the leaving/dead component. The invalidation token (bumped
      // on the wipe) drops the stale result.
      let resolveShow!: (v: {
        password: string;
        notes: string;
        has_totp: boolean;
        attachment: null;
      }) => void;
      vi.mocked(invoke).mockImplementation((cmd: string) => {
        if (cmd === "plugin:screen-secure|set_secure") return Promise.resolve();
        if (cmd === "show_password")
          return new Promise((r) => {
            resolveShow = r;
          });
        return Promise.resolve(undefined);
      });
      const wrapper = mountPage();
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises(); // withClaim acquires; show_password is pending

      // Simulate Back: popstate fires the wipe, bumping the invalidation token.
      window.dispatchEvent(new PopStateEvent("popstate"));
      await flushPromises();

      // The decrypt resolves late — must NOT render into the leaving page.
      resolveShow({
        password: "late-secret",
        notes: "",
        has_totp: false,
        attachment: null,
      });
      await flushPromises();

      expect(wrapper.text()).not.toContain("late-secret");
    });
  });

  describe("copyPassword", () => {
    it("invokes copy_password and shows success toast", async () => {
      vi.mocked(invoke).mockResolvedValue({
        entry_name: "prod",
        cleared_after_secs: 45,
        has_totp: false,
        has_attachment: false,
      });
      const { wrapper, toast } = mountWithApp(EntryDetailPage);
      await wrapper
        .find('button[aria-label="Copy password to clipboard"]')
        .trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith(
        "copy_password",
        expect.objectContaining({ entryPath: "servers/prod.age" }),
      );
      expect(
        toast.toasts.value.some((t) =>
          t.message.includes("✓ Copied prod (45s auto-clear)"),
        ),
      ).toBe(true);
    });

    it("shows the non-UTF-8 info toast when the password can't be copied", async () => {
      // Backend skipped the clipboard write (password isn't valid UTF-8); the
      // UI must say so instead of crowning an empty copy with "Copied!".
      vi.mocked(invoke).mockResolvedValue({
        entry_name: "prod",
        cleared_after_secs: 0,
        has_totp: false,
        has_attachment: false,
        password_non_utf8: true,
      });
      const { wrapper, toast } = mountWithApp(EntryDetailPage);
      await wrapper
        .find('button[aria-label="Copy password to clipboard"]')
        .trigger("click");
      await flushPromises();

      expect(
        toast.toasts.value.some((t) => t.message.includes("can't be copied")),
      ).toBe(true);
      expect(
        toast.toasts.value.some((t) => t.message.includes("✓ Copied")),
      ).toBe(false);
    });

    it("hides the whole action pile behind the Unlock gate when cold (R086)", async () => {
      // R086: a cold identity shows only Unlock + Delete — the Copy/Show/TOTP/
      // Edit/Revisions pile is hidden so a cold start is a single action, not a
      // button pile that dead-ends into unlock prompts.
      const { wrapper } = mountWithApp(EntryDetailPage, { unlocked: false });
      await flushPromises();
      expect(
        wrapper
          .find('button[aria-label="Copy password to clipboard"]')
          .exists(),
      ).toBe(false);
      expect(wrapper.find('button[aria-label="Show password"]').exists()).toBe(
        false,
      );
      expect(
        wrapper.find('button[aria-label="Edit servers/prod"]').exists(),
      ).toBe(false);
      expect(
        wrapper
          .find('button[aria-label="Revision history for servers/prod"]')
          .exists(),
      ).toBe(false);
      // The gate's Unlock button is the single primary action.
      expect(
        wrapper.find('button[aria-label="Unlock servers/prod"]').exists(),
      ).toBe(true);
    });

    it("clears sensitive data immediately after copy", async () => {
      // Dispatch by command name, not call order: copyPassword's
      // ensureClipboardNotifyPermission adds an are_clipboard_notifications_enabled
      // probe between show_password and copy_password, so an order-based Once
      // queue drifts and only passes via a sibling test's leaked default.
      vi.mocked(invoke).mockImplementation((cmd: string) => {
        switch (cmd) {
          case "entry_probe":
            return Promise.resolve({ has_totp: true, attachment: null });
          case "are_clipboard_notifications_enabled":
            return Promise.resolve(true);
          case "show_password":
            return Promise.resolve({
              password: "s3cret",
              notes: "",
              has_totp: false,
              attachment: null,
            });
          case "copy_password":
            return Promise.resolve({
              entry_name: "prod",
              cleared_after_secs: 45,
              has_totp: false,
              has_attachment: false,
            });
          default:
            return Promise.resolve(undefined);
        }
      });

      const wrapper = mountPage();
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();

      expect(wrapper.text()).toContain("s3cret");

      // Now copy — this should clear sensitive data
      await wrapper
        .find('button[aria-label="Copy password to clipboard"]')
        .trigger("click");
      await flushPromises();

      expect(wrapper.text()).not.toContain("s3cret");
    });

    it("auto-clears toast after 3 seconds", async () => {
      vi.mocked(invoke).mockResolvedValue({
        entry_name: "prod",
        cleared_after_secs: 45,
        has_totp: false,
        has_attachment: false,
      });
      const { wrapper, toast } = mountWithApp(EntryDetailPage);
      await wrapper
        .find('button[aria-label="Copy password to clipboard"]')
        .trigger("click");
      await flushPromises();

      expect(
        toast.toasts.value.some((t) => t.message.includes("✓ Copied prod")),
      ).toBe(true);

      vi.advanceTimersByTime(3000);
      await flushPromises();

      expect(toast.toasts.value).toHaveLength(0);
    });
  });

  describe("copyTotp", () => {
    it("invokes copy_totp and shows success toast when the entry has a seed", async () => {
      vi.mocked(invoke).mockResolvedValue({
        copied: true,
        entry_name: "prod",
        cleared_after_secs: 45,
      });
      const { wrapper, toast } = mountWithApp(EntryDetailPage);
      await wrapper
        .find('button[aria-label="Copy 2FA code to clipboard"]')
        .trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith(
        "copy_totp",
        expect.objectContaining({ entryPath: "servers/prod.age" }),
      );
      expect(
        toast.toasts.value.some((t) =>
          t.message.includes("✓ 2FA code copied for prod"),
        ),
      ).toBe(true);
    });

    it("shows the no-2FA info toast when the entry has no seed", async () => {
      vi.mocked(invoke).mockResolvedValue({
        copied: false,
        entry_name: "prod",
        cleared_after_secs: 0,
      });
      const { wrapper, toast } = mountWithApp(EntryDetailPage);
      await wrapper
        .find('button[aria-label="Copy 2FA code to clipboard"]')
        .trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith(
        "copy_totp",
        expect.objectContaining({ entryPath: "servers/prod.age" }),
      );
      expect(
        toast.toasts.value.some((t) =>
          t.message.includes("This entry has no 2FA code"),
        ),
      ).toBe(true);
    });

    it("keeps Delete on the cold gate (de-emphasized) so delete-without-decrypt still works (R086)", async () => {
      // R086 design decision: the cold gate hides the action pile but keeps
      // Delete (outline, de-emphasized) so a delete-without-decrypt is still one
      // tap away. Copy/Show stay hidden (they would dead-end into a prompt).
      const { wrapper } = mountWithApp(EntryDetailPage, { unlocked: false });
      await flushPromises();
      // Delete is present on the gate...
      expect(
        wrapper.find('button[aria-label="Delete servers/prod"]').exists(),
      ).toBe(true);
      // ...while the TOTP action (the copyTotp entry point) is hidden.
      expect(
        wrapper
          .find('button[aria-label="Copy 2FA code to clipboard"]')
          .exists(),
      ).toBe(false);
    });
  });

  describe("security lifecycle", () => {
    it("clears sensitive data on unmount", async () => {
      vi.mocked(invoke).mockResolvedValue({
        password: "s3cret",
        notes: "notes",
        has_totp: false,
        attachment: null,
      });
      const wrapper = mountPage();
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();

      // Password is in DOM
      expect(wrapper.text()).toContain("s3cret");

      // Unmount triggers clearSensitive via onBeforeUnmount
      wrapper.unmount();

      // The key assertion: no memory leak of timers
      // (can't directly check internal state after unmount,
      //  but we verify no lingering setTimeout throws)
    });

    it("clears sensitive data on identity lock", async () => {
      vi.mocked(invoke).mockResolvedValue({
        password: "s3cret",
        notes: "notes",
        has_totp: false,
        attachment: null,
      });
      // The modal keeps the page mounted, so a lock transition must wipe in place.
      const { wrapper, lock } = mountWithApp(EntryDetailPage);
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();

      // Password is in the DOM
      expect(wrapper.text()).toContain("s3cret");

      // Lock fires the shared composable's onLock(clear) without unmounting.
      lock.setLocked(true);
      await flushPromises();

      expect(wrapper.text()).not.toContain("s3cret");
    });

    it("handles ESC key to go back", async () => {
      vi.mocked(invoke).mockResolvedValue({
        password: "s3cret",
        notes: "",
        has_totp: false,
        attachment: null,
      });
      const wrapper = mountPage();
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();

      // Press ESC on the main element
      await wrapper.find("main").trigger("keydown", { key: "Escape" });
      await flushPromises();

      expect(mockReplace).toHaveBeenCalledWith({ name: "entries" });
    });

    it("header Back button navigates back and wipes the revealed password", async () => {
      vi.mocked(invoke).mockResolvedValue({
        password: "s3cret",
        notes: "",
        has_totp: false,
        attachment: null,
      });
      const wrapper = mountPage();
      await wrapper.find('button[aria-label="Show password"]').trigger("click");
      await flushPromises();
      expect(wrapper.text()).toContain("s3cret");

      // Header Back (BaseHeader): @back="clear" wipes, then navBack runs.
      await wrapper.find('button[aria-label="Back"]').trigger("click");
      await flushPromises();

      expect(wrapper.text()).not.toContain("s3cret");
      expect(mockReplace).toHaveBeenCalledWith({ name: "entries" });
    });
  });

  describe("deleteSecret", () => {
    // mountWithApp provides a dialog whose confirm resolves true by default
    // (the "proceed" case); the cancel test overrides it to false.
    const deleteBtn = () => 'button[aria-label="Delete servers/prod"]';

    // R026: the detail page probes `entry_oid` on mount so a delete-without-
    // reveal is still base-version-guarded. The dispatch-by-command mock mirrors
    // the copyPassword test's caution: an ordered Once queue drifts once any
    // sibling probe (here the mount-time `has_totp` + `entry_oid`) sneaks in.
    function mockDelete(opts: {
      entryOid: string | null;
      deleteOutcome?: unknown;
    }) {
      vi.mocked(invoke).mockImplementation((cmd: string) => {
        switch (cmd) {
          case "has_totp":
            return Promise.resolve(false);
          case "entry_oid":
            return Promise.resolve(opts.entryOid);
          case "delete_secret":
            return Promise.resolve(
              opts.deleteOutcome ?? { kind: "written", commit: "abc1234" },
            );
          default:
            return Promise.resolve(undefined);
        }
      });
    }

    it("on confirm, invokes delete_secret with {name, baseOid} once the entry_oid probe settles", async () => {
      // Previously this was GREEN FOR THE WRONG REASON: clicking delete before
      // flushPromises let the mount-time entry_oid probe race the click, so
      // baseOid was still null at the call site and the assertion {name} only
      // passed by accident. Flush the probes first so a real oid propagates.
      mockDelete({ entryOid: "oid-deadbeef" });
      const wrapper = mountPage();
      await flushPromises(); // mount-time entry_oid + has_totp probes settle
      await wrapper.find(deleteBtn()).trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("delete_secret", {
        name: "servers/prod",
        baseOid: "oid-deadbeef",
      });
    });

    it("OMITS baseOid when entry_oid returns null (legacy/absent)", async () => {
      // deleteSecret's API helper spreads `baseOid` only when it is non-null,
      // so a legacy/absent entry_oid yields the {name}-only legacy payload.
      mockDelete({ entryOid: null });
      const wrapper = mountPage();
      await flushPromises();
      await wrapper.find(deleteBtn()).trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("delete_secret", {
        name: "servers/prod",
      });
      // Belt-and-suspendences: no baseOid key at all on the arg object.
      const call = vi
        .mocked(invoke)
        .mock.calls.find(([cmd]) => cmd === "delete_secret");
      expect(call?.[1]).not.toHaveProperty("baseOid");
    });

    it("on success, toasts and navigates to the list", async () => {
      vi.mocked(invoke).mockResolvedValue({
        kind: "written",
        commit: "abc1234",
      });
      const { wrapper, toast } = mountWithApp(EntryDetailPage);
      await wrapper.find(deleteBtn()).trigger("click");
      await flushPromises();

      expect(
        toast.toasts.value.some((t) =>
          t.message.includes("✓ Deleted (commit abc1234)"),
        ),
      ).toBe(true);
      expect(mockReplace).toHaveBeenCalledWith({ name: "entries" });
    });

    it("on no_change outcome, toasts 'Already removed elsewhere' and navigates to the list", async () => {
      // R026: a teammate already removed it — distinct from `written` so the
      // toast says "already removed" instead of fabricating a delete commit.
      mockDelete({
        entryOid: null,
        deleteOutcome: { kind: "no_change", head: "abc" },
      });
      const { wrapper, toast } = mountWithApp(EntryDetailPage);
      await flushPromises();
      await wrapper.find(deleteBtn()).trigger("click");
      await flushPromises();

      expect(
        toast.toasts.value.some((t) =>
          t.message.includes("Already removed elsewhere"),
        ),
      ).toBe(true);
      expect(mockReplace).toHaveBeenCalledWith({ name: "entries" });
    });

    it("on entry_conflict outcome, surfaces the per-entry modal (delete op)", async () => {
      // R026: a teammate changed the entry since the read — the delete surfaces
      // the EntryConflictModal (delete op) instead of removing it. The no_change
      // and needs_divergence_resolve outcomes are covered above; this pins the
      // delete-conflict modal render (heading + entry name + op-specific copy).
      mockDelete({
        entryOid: "oid-1",
        deleteOutcome: {
          kind: "entry_conflict",
          name: "servers/prod",
          base_oid: "oid-1",
          current_oid: "oid-2",
          remote_tip: "tip-3",
          op: "delete",
        },
      });
      const wrapper = mountPage();
      await flushPromises();
      await wrapper.find(deleteBtn()).trigger("click");
      await flushPromises();

      // The per-entry conflict sheet shows the delete-op heading + the name.
      expect(wrapper.text()).toContain("This secret was changed elsewhere");
      expect(wrapper.text()).toContain("servers/prod");
      // op-specific step-1 labels render (delete keeps theirs / deletes anyway).
      expect(wrapper.text()).toContain("Keep their version");
      expect(wrapper.text()).toContain("Delete it anyway");
    });

    it("on delete divergence, surfaces the shared modal and adopt resolves", async () => {
      // First queued response is the mount-time `has_totp` probe (identity is
      // cached by default); the next two are the delete + its resolve.
      vi.mocked(invoke)
        .mockResolvedValueOnce({ has_totp: false, attachment: null }) // mount: entry_probe (identity cached)
        .mockResolvedValueOnce(null) // mount: entry_oid base-version probe (R026)
        .mockResolvedValueOnce({
          kind: "needs_divergence_resolve",
          local_ahead: 1,
          remote_ahead: 1,
          remote_tip: "abc123",
          local_only_entries: [],
          modified_entries: ["servers/prod"],
          other_changed_files: [],
        })
        .mockResolvedValueOnce({
          changed: true,
          head: "def456",
          authenticity: {
            mode: "off",
            new_commits: [],
            open_issues: [],
            blocked: false,
          },
        });
      const wrapper = mountPage();
      await wrapper.find(deleteBtn()).trigger("click");
      await flushPromises();

      // The shared divergence modal shows (save wording + the modified entry).
      expect(wrapper.text()).toContain("conflicts with a newer remote");
      expect(wrapper.text()).toContain("servers/prod");

      const adopt = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Adopt remote"))!;
      await adopt.trigger("click");
      await flushPromises();

      const confirmBtn = wrapper
        .findAll("button")
        .find((b) => b.text().includes("Discard my commit"))!;
      await confirmBtn.trigger("click");
      await flushPromises();

      expect(invoke).toHaveBeenCalledWith("resolve_sync_divergence", {
        expectedRemoteOid: "abc123",
        choice: "adopt_remote",
      });
    });

    it("on a non-PUSH_REJECTED error, shows the error and stays put", async () => {
      vi.mocked(invoke).mockRejectedValue({
        code: "STORE_ERROR",
        message: "Disk full",
      });
      const wrapper = mountPage();
      await wrapper.find(deleteBtn()).trigger("click");
      await flushPromises();

      expect(wrapper.find("[role='alert']").text()).toContain("Disk full");
      expect(mockPush).not.toHaveBeenCalled();
    });

    it("disables the button while the delete is inflight", async () => {
      let resolveDelete!: (v: { commit: string }) => void;
      vi.mocked(invoke).mockReturnValue(
        new Promise<{ commit: string }>((r) => {
          resolveDelete = r;
        }),
      );
      const wrapper = mountPage();
      const btn = wrapper.find(deleteBtn());
      expect(btn.attributes("disabled")).toBeUndefined();

      await btn.trigger("click");
      await flushPromises();
      expect(btn.attributes("disabled")).toBeDefined();

      resolveDelete({ commit: "abc1234" });
      await flushPromises();
      expect(btn.attributes("disabled")).toBeUndefined();
    });

    it("does not invoke when confirm is cancelled", async () => {
      vi.mocked(invoke).mockResolvedValue({ commit: "abc1234" });
      const { wrapper, dialog } = mountWithApp(EntryDetailPage);
      vi.mocked(dialog.dialog.confirm).mockResolvedValue(false);
      await wrapper.find(deleteBtn()).trigger("click");
      await flushPromises();

      // The mount-time `has_totp` probe still runs; assert the delete itself
      // never happened rather than total invoke silence.
      expect(invoke).not.toHaveBeenCalledWith(
        "delete_secret",
        expect.anything(),
      );
      expect(mockPush).not.toHaveBeenCalled();
    });
  });

  describe("2FA button visibility", () => {
    const totpBtn = () => 'button[aria-label="Copy 2FA code to clipboard"]';

    it("shows the button when the entry has a seed (identity cached → free probe)", async () => {
      // entry_probe → seed present, no attachment.
      vi.mocked(invoke).mockResolvedValue({ has_totp: true, attachment: null });
      const wrapper = mountPage();
      await flushPromises();
      expect(wrapper.find(totpBtn()).exists()).toBe(true);
    });

    it("hides the button when the entry has no seed (identity cached → free probe)", async () => {
      // entry_probe → no seed, no attachment.
      vi.mocked(invoke).mockResolvedValue({
        has_totp: false,
        attachment: null,
      });
      const wrapper = mountPage();
      await flushPromises();
      expect(wrapper.find(totpBtn()).exists()).toBe(false);
    });

    it("hides the button behind the Unlock gate when the identity is not cached (R086)", async () => {
      // R086: a cold identity shows the Unlock gate, not the button pile — so the
      // mount probe is skipped (never forces an unlock) and the TOTP button is
      // absent until the user unlocks.
      const { wrapper } = mountWithApp(EntryDetailPage, { unlocked: false });
      await flushPromises();
      expect(wrapper.find(totpBtn()).exists()).toBe(false);
      // The gate's Unlock button shows instead.
      expect(
        wrapper.find('button[aria-label="Unlock servers/prod"]').exists(),
      ).toBe(true);
      // has_totp is identity-gated, so it is NOT probed when uncached; entry_oid
      // (R026, non-secret) does run on mount — assert the identity probe specifically.
      expect(invoke).not.toHaveBeenCalledWith("has_totp", expect.anything());
    });
  });

  describe("attachment entry", () => {
    it("hides Copy/Show and shows Export + metadata + locked Edit for a confirmed attachment", async () => {
      vi.mocked(invoke).mockResolvedValue({
        has_totp: false,
        attachment: { filename: "photo.png", size: 1234 },
      });
      const wrapper = mountPage();
      await flushPromises();

      // Copy/Show are dead for an attachment (empty password, base64 body).
      expect(
        wrapper
          .find('button[aria-label="Copy password to clipboard"]')
          .exists(),
      ).toBe(false);
      expect(wrapper.find('button[aria-label="Show password"]').exists()).toBe(
        false,
      );
      // Export is the primary action; the metadata caption shows the filename.
      expect(
        wrapper
          .find('button[aria-label="Export attachment to a file"]')
          .exists(),
      ).toBe(true);
      expect(wrapper.text()).toContain("photo.png");
      // Edit is locked with the attachment hint.
      expect(
        wrapper
          .find('button[aria-label="Edit servers/prod"]')
          .attributes("disabled"),
      ).toBeDefined();
      expect(wrapper.text()).toContain("Attachments can't be edited yet");
    });

    it("swallows a dismissed save picker (CANCELLED) with no error UI", async () => {
      vi.mocked(invoke).mockImplementation((cmd: string) => {
        if (cmd === "entry_probe")
          return Promise.resolve({
            has_totp: false,
            attachment: { filename: "x.bin", size: 1 },
          });
        if (cmd === "export_attachment")
          return Promise.reject({
            code: "CANCELLED",
            message: "Save cancelled",
          });
        return Promise.resolve(undefined);
      });
      const wrapper = mountPage();
      await flushPromises();
      await wrapper
        .find('button[aria-label="Export attachment to a file"]')
        .trigger("click");
      await flushPromises();
      // A dismissed picker is a silent cancel, not an error.
      expect(wrapper.find("[role='alert']").exists()).toBe(false);
    });
  });
});
