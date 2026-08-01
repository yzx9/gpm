// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { __resetLicensesCacheForTests } from "@/components/about/data";
import { mountWithApp } from "@/test/appTestUtils";
import { flushPromises } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import pkg from "../../package.json";
import AboutPage from "./AboutPage.vue";

beforeEach(() => {
  __resetLicensesCacheForTests();
  // Licenses tab fetches on mount; stub a minimal valid response.
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue({
      ok: true,
      json: () =>
        Promise.resolve({
          generatedAt: null,
          complete: true,
          note: "",
          ecosystems: {},
          packages: [],
        }),
    }),
  );
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("AboutPage", () => {
  it("defaults to the Overview tab", async () => {
    const { wrapper } = mountWithApp(AboutPage);
    await flushPromises();
    // Overview identity card: the app name (first <h2>) and the version.
    expect(wrapper.find("h2").text()).toContain("gpm");
    expect(wrapper.text()).toContain(pkg.version);
    expect(wrapper.text()).toContain("Design goals");
  });

  it("switches to the Licenses tab", async () => {
    const { wrapper } = mountWithApp(AboutPage);
    await flushPromises();
    // Overview is up first; the search box is not yet present.
    expect(wrapper.find('input[type="search"]').exists()).toBe(false);

    // Select the 3rd pill (Licenses) by firing `change` on its radio directly
    // — deterministic vs. relying on jsdom label→control activation.
    const radios = wrapper.findAll('input[name="about-tabs"]');
    await radios[2].trigger("change");
    await flushPromises();

    // Licenses tab renders its search input.
    expect(wrapper.find('input[type="search"]').exists()).toBe(true);
  });

  it("switches to the Acknowledgements tab", async () => {
    const { wrapper } = mountWithApp(AboutPage);
    await flushPromises();
    const radios = wrapper.findAll('input[name="about-tabs"]');
    await radios[1].trigger("change");
    await flushPromises();
    // gopass is the first/primary acknowledgement.
    expect(wrapper.text()).toContain("gopass");
    // a11y: external ack links announce they open a new window (WCAG G201).
    expect(wrapper.text()).toContain("opens in a new window");
  });
});
