// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount } from "@vue/test-utils";
import { flushPromises } from "@vue/test-utils";
import { invoke } from "@tauri-apps/api/core";
import SetupPage from "./SetupPage.vue";

const { mockPush } = vi.hoisted(() => ({
  mockPush: vi.fn(),
}));

vi.mock("@tauri-apps/api/core");
vi.mock("vue-router", () => ({
  createRouter: vi.fn(),
  createWebHashHistory: vi.fn(),
  useRouter: () => ({
    push: mockPush,
    replace: vi.fn(),
    back: vi.fn(),
  }),
  useRoute: () => ({
    params: {},
    query: {},
    name: "",
    path: "/",
    fullPath: "/",
  }),
}));

describe("SetupPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  async function fillForm(
    wrapper: ReturnType<typeof mount>,
    opts: { repoUrl?: string; pat?: string; identity?: string } = {},
  ) {
    const defaults = {
      repoUrl: "https://github.com/user/passwords.git",
      pat: "",
      identity: "AGE-SECRET-KEY-1abc123def456",
    };
    const vals = { ...defaults, ...opts };

    if (vals.repoUrl !== undefined) {
      await wrapper.find('input[id="repo-url"]').setValue(vals.repoUrl);
    }
    if (vals.pat !== undefined) {
      await wrapper.find('input[id="pat"]').setValue(vals.pat);
    }
    if (vals.identity !== undefined) {
      await wrapper.find("textarea").setValue(vals.identity);
    }
  }

  async function submitForm(wrapper: ReturnType<typeof mount>) {
    await wrapper.find("form").trigger("submit.prevent");
    await flushPromises();
  }

  describe("validation", () => {
    it("shows error when repo URL is empty", async () => {
      const wrapper = mount(SetupPage);
      await fillForm(wrapper, { repoUrl: "" });
      await submitForm(wrapper);

      expect(wrapper.find("[role='alert']").text()).toBe(
        "Repository URL is required",
      );
      expect(invoke).not.toHaveBeenCalled();
    });

    it("shows error for non-HTTPS URL", async () => {
      const wrapper = mount(SetupPage);
      await fillForm(wrapper, { repoUrl: "http://github.com/user/repo.git" });
      await submitForm(wrapper);

      expect(wrapper.find("[role='alert']").text()).toBe(
        "Only HTTPS URLs are supported",
      );
    });

    it("shows error when identity is empty", async () => {
      const wrapper = mount(SetupPage);
      await fillForm(wrapper, {
        repoUrl: "https://github.com/user/repo.git",
        identity: "",
      });
      await submitForm(wrapper);

      expect(wrapper.find("[role='alert']").text()).toBe(
        "Age identity is required",
      );
    });

    it("shows error when identity lacks AGE-SECRET-KEY- prefix", async () => {
      const wrapper = mount(SetupPage);
      await fillForm(wrapper, { identity: "not-a-valid-key" });
      await submitForm(wrapper);

      expect(wrapper.find("[role='alert']").text()).toBe(
        "Identity must start with AGE-SECRET-KEY-...",
      );
    });
  });

  describe("successful submission", () => {
    it("calls invoke with correct args", async () => {
      vi.mocked(invoke).mockResolvedValue(undefined);
      const wrapper = mount(SetupPage);
      await fillForm(wrapper, {
        repoUrl: "https://github.com/user/repo.git",
        pat: "my-token",
        identity: "AGE-SECRET-KEY-1abc",
      });
      await submitForm(wrapper);

      expect(invoke).toHaveBeenCalledWith("setup", {
        repoUrl: "https://github.com/user/repo.git",
        pat: "my-token",
        identity: "AGE-SECRET-KEY-1abc",
      });
    });

    it("passes null for pat when empty", async () => {
      vi.mocked(invoke).mockResolvedValue(undefined);
      const wrapper = mount(SetupPage);
      await fillForm(wrapper, { pat: "" });
      await submitForm(wrapper);

      expect(invoke).toHaveBeenCalledWith(
        "setup",
        expect.objectContaining({ pat: null }),
      );
    });

    it("navigates to entries on success", async () => {
      vi.mocked(invoke).mockResolvedValue(undefined);
      const wrapper = mount(SetupPage);
      await fillForm(wrapper);
      await submitForm(wrapper);

      expect(mockPush).toHaveBeenCalledWith({ name: "entries" });
    });
  });

  describe("error handling", () => {
    it("displays error from AppError", async () => {
      vi.mocked(invoke).mockRejectedValue({
        code: "SetupFailed",
        message: "Clone failed",
      });
      const wrapper = mount(SetupPage);
      await fillForm(wrapper);
      await submitForm(wrapper);

      expect(wrapper.find("[role='alert']").text()).toBe("Clone failed");
    });

    it("falls back to 'Setup failed' on unknown error", async () => {
      vi.mocked(invoke).mockRejectedValue(null);
      const wrapper = mount(SetupPage);
      await fillForm(wrapper);
      await submitForm(wrapper);

      expect(wrapper.find("[role='alert']").text()).toBe("Setup failed");
    });
  });
});
