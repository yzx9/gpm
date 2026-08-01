// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { mountWithApp } from "@/test/appTestUtils";
import { flushPromises } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import SecurityPage from "./SecurityPage.vue";

// Pure static page: the `security` en bundle is inlined in src/test/i18n.ts, so
// t() resolves without the (mocked, non-navigating) router firing the real
// bundle auto-load. The footer link is asserted by presence only —
// @tauri-apps/plugin-opener isn't mocked in setup.ts, so clicking would throw
// into the window.open fallback (same reason AboutPage.test never clicks).
describe("SecurityPage", () => {
  function mountPage() {
    return mountWithApp(SecurityPage).wrapper;
  }

  it("renders the title, intro, and all seven card titles", async () => {
    const wrapper = mountPage();
    await flushPromises();

    expect(wrapper.find("h1").text()).toContain("Security");

    const text = wrapper.text();
    expect(text).toContain("A plain-language look");
    // All seven card titles render (substring presence; order isn't asserted).
    expect(text).toContain("Secrets stay on your device");
    expect(text).toContain("Copy vs. show");
    expect(text).toContain("The key is wiped after every use");
    expect(text).toContain("Encrypted at rest on Android");
    expect(text).toContain("Optional App Lock");
    expect(text).toContain("Optional signature verification");
    expect(text).toContain("What this protects against");
  });

  it("renders the full-model link pointing at the repo SECURITY.md", async () => {
    const wrapper = mountPage();
    await flushPromises();

    const link = wrapper.find('[data-testid="security-full-model-link"]');
    expect(link.exists()).toBe(true);
    expect(link.attributes("href")).toBe(
      "https://github.com/yzx9/gpm/blob/main/docs/SECURITY.md",
    );
    expect(link.text()).toContain("Read the full security model");
    // a11y: the link announces that it opens a new window (WCAG G201).
    expect(link.find(".sr-only").text()).toBe("opens in a new window");
  });
});
