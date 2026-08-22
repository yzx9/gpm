// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { mountWithApp } from "@/test/appTestUtils";
import { flushPromises } from "@vue/test-utils";
import { describe, expect, it, vi } from "vitest";
import AcknowledgementsPage from "./AcknowledgementsPage.vue";

const { mockReplace } = vi.hoisted(() => ({ mockReplace: vi.fn() }));

vi.mock("vue-router", () => ({
  createRouter: vi.fn(),
  createWebHashHistory: vi.fn(),
  onBeforeRouteLeave: vi.fn(),
  useRouter: () => ({ push: vi.fn(), replace: mockReplace, back: vi.fn() }),
  useRoute: () => ({
    params: {},
    query: {},
    name: "",
    path: "/",
    fullPath: "/",
  }),
}));

describe("AcknowledgementsPage", () => {
  it("renders the title and the acknowledgements list", async () => {
    const { wrapper } = mountWithApp(AcknowledgementsPage);
    await flushPromises();
    expect(wrapper.find("h1").text()).toBe("Acknowledgements");
    // gopass is the first/primary acknowledgement.
    expect(wrapper.text()).toContain("gopass");
    // a11y: external ack links announce they open a new window (WCAG G201).
    expect(wrapper.text()).toContain("opens in a new window");
  });

  it("navigates back to the Settings hub when Back is clicked", async () => {
    const { wrapper } = mountWithApp(AcknowledgementsPage);
    await flushPromises();

    await wrapper.find('button[aria-label="Back"]').trigger("click");

    // navBack falls back to replace when there is no history to pop.
    expect(mockReplace).toHaveBeenCalledWith({ name: "settings" });
  });
});
