// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { mountWithApp } from "@/test/appTestUtils";
import { invoke } from "@tauri-apps/api/core";
import { flushPromises } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import GeneratePasswordPage from "./GeneratePasswordPage.vue";

const { mockPush, mockReplace } = vi.hoisted(() => ({
  mockPush: vi.fn(),
  mockReplace: vi.fn(),
}));

vi.mock("@tauri-apps/api/core");

vi.mock("vue-router", () => ({
  createRouter: vi.fn(),
  createWebHashHistory: vi.fn(),
  useRouter: () => ({ push: mockPush, replace: mockReplace, back: vi.fn() }),
}));

/** Wire `invoke` per command; returns a fresh mounted GeneratePasswordPage. */
async function mountPage(handlers: Record<string, () => unknown>) {
  vi.mocked(invoke).mockImplementation(((cmd: string) => {
    const h = handlers[cmd];
    if (h) return Promise.resolve(h());
    return Promise.resolve(undefined);
  }) as typeof invoke);
  const app = mountWithApp(GeneratePasswordPage);
  await flushPromises();
  return app;
}

describe("GeneratePasswordPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("generates 10 passwords by default", async () => {
    const { wrapper } = await mountPage({
      generate_password_batch: () => ["a"],
    });
    await wrapper.find("form").trigger("submit");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith(
      "generate_password_batch",
      expect.objectContaining({ count: 10 }),
    );
  });

  it("renders one row per generated password", async () => {
    const { wrapper } = await mountPage({
      generate_password_batch: () => ["aa", "bb", "cc"],
    });
    await wrapper.find("#g-count").setValue("3");
    await wrapper.find("form").trigger("submit");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith(
      "generate_password_batch",
      expect.objectContaining({ count: 3 }),
    );
    expect(wrapper.findAll(".result-row")).toHaveLength(3);
  });

  it("the style selector changes which generator runs", async () => {
    const { wrapper } = await mountPage({
      generate_password_batch: () => ["x"],
    });
    await wrapper.find('select[aria-label="Password style"]').setValue("xkcd");
    await wrapper.find("form").trigger("submit");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith(
      "generate_password_batch",
      expect.objectContaining({ mode: "xkcd" }),
    );
  });

  it("random mode pins an exact length via min == max", async () => {
    const { wrapper } = await mountPage({
      generate_password_batch: () => ["x"],
    });
    // Defaults: mode random, length 24.
    await wrapper.find("form").trigger("submit");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith(
      "generate_password_batch",
      expect.objectContaining({ mode: "random", minLen: 24, maxLen: 24 }),
    );
  });

  it("a chosen length flows through to the request", async () => {
    const { wrapper } = await mountPage({
      generate_password_batch: () => ["x"],
    });
    await wrapper.find("#g-length").setValue("32");
    await wrapper.find("form").trigger("submit");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith(
      "generate_password_batch",
      expect.objectContaining({ minLen: 32, maxLen: 32 }),
    );
  });

  it("xkcd hides the length control and sends no length", async () => {
    const { wrapper } = await mountPage({
      generate_password_batch: () => ["x"],
    });
    await wrapper.find('select[aria-label="Password style"]').setValue("xkcd");
    await flushPromises();

    expect(wrapper.find("#g-length").exists()).toBe(false);

    await wrapper.find("form").trigger("submit");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith(
      "generate_password_batch",
      expect.objectContaining({ minLen: null, maxLen: null }),
    );
  });

  it("copying a row invokes copy_generated_password and toasts", async () => {
    const { wrapper, toast } = await mountPage({
      generate_password_batch: () => ["topsecret"],
      copy_generated_password: () => undefined,
    });
    await wrapper.find("form").trigger("submit");
    await flushPromises();

    await wrapper.find('button[aria-label="Copy"]').trigger("click");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith(
      "copy_generated_password",
      expect.objectContaining({ text: "topsecret" }),
    );
    expect(toast.toasts.value.some((t) => t.message.includes("Copied"))).toBe(
      true,
    );
  });

  it("a generate error shows the message and renders no rows", async () => {
    vi.mocked(invoke).mockImplementation(((cmd: string) => {
      if (cmd === "generate_password_batch") {
        return Promise.reject({ code: "STORE_ERROR", message: "RNG down" });
      }
      return Promise.resolve(undefined);
    }) as typeof invoke);
    const { wrapper } = mountWithApp(GeneratePasswordPage);
    await flushPromises();

    await wrapper.find("form").trigger("submit");
    await flushPromises();

    expect(wrapper.text()).toContain("RNG down");
    expect(wrapper.findAll(".result-row")).toHaveLength(0);
  });

  it("clears the batch the moment the identity locks", async () => {
    vi.mocked(invoke).mockImplementation(((cmd: string) => {
      if (cmd === "generate_password_batch")
        return Promise.resolve(["a", "b", "c"]);
      return Promise.resolve(undefined);
    }) as typeof invoke);
    const { wrapper, lock } = mountWithApp(GeneratePasswordPage);
    await flushPromises();
    await wrapper.find("form").trigger("submit");
    await flushPromises();
    expect(wrapper.findAll(".result-row")).toHaveLength(3);

    // The page registers an onLock clearer via useLockState; drive the same
    // unlocked → locked transition the backend event would (default mount is
    // unlocked, so a single setLocked(true) fires the clearer).
    lock.setLocked(true);
    await flushPromises();

    expect(wrapper.findAll(".result-row")).toHaveLength(0);
  });

  it("clears the batch on browser back (popstate)", async () => {
    vi.mocked(invoke).mockImplementation(((cmd: string) => {
      if (cmd === "generate_password_batch")
        return Promise.resolve(["a", "b", "c"]);
      return Promise.resolve(undefined);
    }) as typeof invoke);
    const { wrapper } = mountWithApp(GeneratePasswordPage);
    await flushPromises();
    await wrapper.find("form").trigger("submit");
    await flushPromises();
    expect(wrapper.findAll(".result-row")).toHaveLength(3);

    // vue-router is mocked, so drive popstate directly (the real browser-back path).
    window.dispatchEvent(new PopStateEvent("popstate"));
    await flushPromises();

    expect(wrapper.findAll(".result-row")).toHaveLength(0);
  });

  it("drops a stale generate result superseded by a newer one", async () => {
    let resolveFirst!: (v: string[]) => void;
    const firstCall = new Promise<string[]>((r) => {
      resolveFirst = r;
    });
    let calls = 0;
    vi.mocked(invoke).mockImplementation(((cmd: string) => {
      if (cmd === "generate_password_batch") {
        calls++;
        return calls === 1 ? firstCall : Promise.resolve(["second"]);
      }
      return Promise.resolve(undefined);
    }) as typeof invoke);
    const { wrapper } = mountWithApp(GeneratePasswordPage);
    await flushPromises();

    await wrapper.find("form").trigger("submit"); // first generate, in-flight
    await wrapper.find("form").trigger("submit"); // second supersedes it
    await flushPromises();
    expect(wrapper.findAll(".result-row")).toHaveLength(1);
    expect(wrapper.find(".result-pw").text()).toBe("second");

    resolveFirst(["STALE"]); // the superseded first result resolves late
    await flushPromises();
    expect(wrapper.text()).not.toContain("STALE");
  });

  it("Back returns to the entry list", async () => {
    const { wrapper } = await mountPage({});
    await wrapper.find('button[aria-label="Back"]').trigger("click");
    expect(mockReplace).toHaveBeenCalledWith({ name: "entries" });
  });
});
