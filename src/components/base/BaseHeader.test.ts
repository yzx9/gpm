// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { navBack } from "@/utils/nav";
import { Settings } from "@lucide/vue";
import { mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import BaseHeader from "./BaseHeader.vue";

// Mock navBack so the back-button click is observable in isolation. vue-router
// (useRouter) and vue-i18n (useI18n → common.back = "Back") are mocked/installed
// globally by src/test/setup.ts.
vi.mock("@/utils/nav", () => ({ navBack: vi.fn() }));

describe("BaseHeader", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders a banner header", () => {
    const wrapper = mount(BaseHeader);
    expect(wrapper.find('header[role="banner"]').exists()).toBe(true);
  });

  it("renders an icon-only Back button when backFallback is set", () => {
    const wrapper = mount(BaseHeader, {
      props: { backFallback: { name: "entries" } },
    });
    const back = wrapper.find('button[aria-label="Back"]');
    expect(back.exists()).toBe(true);
    // Icon-only: no visible text beside the icon.
    expect(back.text()).toBe("");
  });

  it("does not render a Back button when backFallback is omitted", () => {
    const wrapper = mount(BaseHeader);
    expect(wrapper.find('button[aria-label="Back"]').exists()).toBe(false);
  });

  it("does not render a Back button when the #nav slot overrides the left cluster", () => {
    const wrapper = mount(BaseHeader, {
      props: { backFallback: { name: "entries" } },
      slots: { nav: '<div class="logo">gpm</div>' },
    });
    expect(wrapper.find('button[aria-label="Back"]').exists()).toBe(false);
    expect(wrapper.find(".logo").exists()).toBe(true);
  });

  it("clicking Back calls navBack with the fallback route", async () => {
    const wrapper = mount(BaseHeader, {
      props: { backFallback: { name: "settings" } },
    });
    await wrapper.find('button[aria-label="Back"]').trigger("click");
    expect(navBack).toHaveBeenCalledTimes(1);
    // [0] = router, [1] = fallback.
    expect(
      (navBack as unknown as ReturnType<typeof vi.fn>).mock.calls[0]![1],
    ).toEqual({ name: "settings" });
  });

  it("emits back BEFORE navBack runs (side-effect handler runs first)", async () => {
    const wrapper = mount(BaseHeader, {
      props: {
        backFallback: { name: "entries" },
        onBack: () => {
          // The page's @back handler must run before BaseHeader navigates.
          expect(navBack).not.toHaveBeenCalled();
        },
      },
    });
    await wrapper.find('button[aria-label="Back"]').trigger("click");
    expect(navBack).toHaveBeenCalledTimes(1);
    expect(wrapper.emitted("back")).toHaveLength(1);
  });

  it("renders the title prop as an <h1>", () => {
    const wrapper = mount(BaseHeader, {
      props: { title: "Settings" },
    });
    const h1 = wrapper.find("h1");
    expect(h1.exists()).toBe(true);
    expect(h1.text()).toBe("Settings");
  });

  it("renders the titleIcon inside the title when provided", () => {
    const wrapper = mount(BaseHeader, {
      props: { title: "Settings", titleIcon: Settings },
    });
    // Lucide renders an <svg>; it only appears when titleIcon is set.
    expect(wrapper.find("h1 svg").exists()).toBe(true);
  });

  it("omits the title icon when titleIcon is not provided", () => {
    const wrapper = mount(BaseHeader, { props: { title: "Settings" } });
    expect(wrapper.find("h1 svg").exists()).toBe(false);
  });

  it("the #title slot overrides the title prop", () => {
    const wrapper = mount(BaseHeader, {
      props: { title: "ignored-prop" },
      slots: { title: '<h1 class="custom">Custom</h1>' },
    });
    expect(wrapper.find("h1.custom").exists()).toBe(true);
    expect(wrapper.text()).not.toContain("ignored-prop");
  });

  it("renders the #actions slot on the right", () => {
    const wrapper = mount(BaseHeader, {
      slots: { actions: '<button id="act">Go</button>' },
    });
    expect(wrapper.find("#act").exists()).toBe(true);
  });

  it("renders no right-cluster wrapper when #actions is absent", () => {
    const wrapper = mount(BaseHeader, {
      props: { backFallback: { name: "entries" } },
    });
    // The actions wrapper carries `shrink-0`; the left cluster does not.
    expect(wrapper.find('[class*="shrink-0"]').exists()).toBe(false);
  });

  it("applies mb-4 for spacing=sm and mb-6 by default", () => {
    const sm = mount(BaseHeader, { props: { spacing: "sm" } });
    expect(sm.find("header").classes()).toContain("mb-4");
    const md = mount(BaseHeader);
    expect(md.find("header").classes()).toContain("mb-6");
  });
});
