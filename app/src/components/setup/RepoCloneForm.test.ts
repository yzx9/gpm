// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import RepoCloneForm from "./RepoCloneForm.vue";

vi.mock("@tauri-apps/api/core");

/// Branch `invoke` by command name. The clone must stay in flight for the whole
/// test so the Cancel button stays mounted, while `cancel_git` is wired per-test.
function mockInvoke(
  handlers: Record<string, (args?: Record<string, unknown>) => unknown>,
) {
  vi.mocked(invoke).mockImplementation(async (cmd: string, args?: unknown) => {
    const h = handlers[cmd];
    if (h) return h(args as Record<string, unknown> | undefined);
    return undefined;
  });
}

/// A promise that never resolves — keeps the `clone_repo` await pending so
/// `loading` holds true and the Cancel button is reachable.
function pending(): Promise<unknown> {
  return new Promise(() => {});
}

function mountForm() {
  // repoUrl is an HTTPS URL so validateStep1() passes on submit; the auth
  // models are required but unused for the cancel path.
  return mountWithApp(RepoCloneForm, {
    mountOpts: {
      props: {
        repoUrl: "https://example.com/repo.git",
        pat: "",
        sshKey: "",
        sshPassphrase: "",
      },
    },
  });
}

async function startClone(wrapper: ReturnType<typeof mountForm>["wrapper"]) {
  await wrapper.find("form").trigger("submit.prevent");
  await flushPromises();
}

describe("RepoCloneForm — cancel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("flips the Cancel button to a disabled Cancelling… on click and fires cancel_git", async () => {
    mockInvoke({ clone_repo: () => pending() });

    const { wrapper } = mountForm();
    await flushPromises();

    // Before submit there is no progress block / cancel button.
    expect(wrapper.find("button.cancel-link").exists()).toBe(false);

    await startClone(wrapper);

    // The clone is in flight → the cancel button is present and enabled.
    const cancelBtn = wrapper.find("button.cancel-link");
    expect(cancelBtn.exists()).toBe(true);
    expect(cancelBtn.attributes("disabled")).toBeUndefined();
    expect(cancelBtn.text()).toContain("Cancel");

    await cancelBtn.trigger("click");
    await flushPromises();

    // The cancel request was sent and the button immediately reflects it.
    expect(invoke).toHaveBeenCalledWith("cancel_git");
    const after = wrapper.find("button.cancel-link");
    expect(after.text()).toContain("Cancelling");
    expect(after.attributes("disabled")).toBeDefined();
  });

  it("surfaces a danger toast when the cancel request itself fails (not swallowed)", async () => {
    mockInvoke({
      clone_repo: () => pending(),
      cancel_git: () => {
        throw { code: "STORE_ERROR", message: "cancel pipe broke" };
      },
    });

    const { wrapper, toast } = mountForm();
    await flushPromises();
    await startClone(wrapper);

    await wrapper.find("button.cancel-link").trigger("click");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("cancel_git");
    // The failed cancel is surfaced rather than silently eaten.
    expect(toast.toasts.value).toHaveLength(1);
    expect(toast.toasts.value[0]!.variant).toBe("danger");
    expect(toast.toasts.value[0]!.message).toContain("cancel pipe broke");
  });
});
