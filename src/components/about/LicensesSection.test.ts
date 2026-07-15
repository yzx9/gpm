// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { __resetLicensesCacheForTests } from "@/components/about/data";
import { mountWithApp } from "@/test/appTestUtils";
import { flushPromises } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import LicensesSection from "./LicensesSection.vue";

// Designed so the two license groups have DIFFERENT counts — that makes the
// group sort (count desc) deterministic instead of relying on alphabetical
// tie-breaking. MIT OR Apache-2.0 = 3 (serde, age, vue), MIT = 1 (tokio).
const sample = {
  generatedAt: null,
  complete: true,
  note: "",
  ecosystems: { rust: 3, npm: 1 },
  packages: [
    {
      ecosystem: "rust",
      name: "serde",
      version: "1.0.0",
      license: "MIT OR Apache-2.0",
      repository: "",
      licenseText: "SERDE LICENSE TEXT",
    },
    {
      ecosystem: "rust",
      name: "tokio",
      version: "1.40.0",
      license: "MIT",
      repository: "",
      licenseText: "TOKIO LICENSE TEXT",
    },
    {
      ecosystem: "rust",
      name: "age",
      version: "0.11.0",
      license: "MIT OR Apache-2.0",
      repository: "",
      licenseText: "AGE LICENSE TEXT",
    },
    {
      ecosystem: "npm",
      name: "vue",
      version: "3.5.0",
      license: "MIT OR Apache-2.0",
      repository: "",
      licenseText: "VUE LICENSE TEXT",
    },
  ],
};

function mockFetchOk(data: unknown) {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue({
      ok: true,
      json: () => Promise.resolve(data),
    }),
  );
}

beforeEach(() => {
  __resetLicensesCacheForTests();
  mockFetchOk(sample);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("LicensesSection", () => {
  it("renders the summary counts after loading", async () => {
    const { wrapper } = mountWithApp(LicensesSection);
    await flushPromises();
    // Assert on the summary element specifically — version strings elsewhere
    // also contain digits, so a whole-text check would give false confidence.
    const summary = wrapper.find(".summary");
    expect(summary.text()).toContain("4 open-source dependencies");
    expect(summary.text()).toContain("3 Rust crates");
    expect(summary.text()).toContain("1 npm package");
  });

  it("groups packages by license (collapsed by default)", async () => {
    const { wrapper } = mountWithApp(LicensesSection);
    await flushPromises();
    // Two distinct license groups.
    const heads = wrapper.findAll(".group-head");
    expect(heads.length).toBe(2);
    // MIT OR Apache-2.0 has 3 packages → listed first.
    expect(heads[0].text()).toContain("MIT OR Apache-2.0");
    expect(heads[0].text()).toContain("3 packages");
    // No package rows until a group is expanded.
    expect(wrapper.findAll(".pkg-row").length).toBe(0);
  });

  it("expands a group to reveal its packages, then a package to reveal text", async () => {
    const { wrapper } = mountWithApp(LicensesSection);
    await flushPromises();
    // Open the first group (MIT OR Apache-2.0: serde, age, vue).
    await wrapper.findAll(".group-head")[0].trigger("click");
    // 3 package rows now visible.
    const rows = wrapper.findAll(".pkg-row");
    expect(rows.length).toBe(3);
    // License text not in DOM yet.
    expect(wrapper.text()).not.toContain("SERDE LICENSE TEXT");
    // Expand the serde row.
    const serdeRow = rows.find((r) => r.text().includes("serde"))!;
    await serdeRow.trigger("click");
    expect(wrapper.text()).toContain("SERDE LICENSE TEXT");
  });

  it("search switches to a flat filtered list", async () => {
    const { wrapper } = mountWithApp(LicensesSection);
    await flushPromises();
    // Groups present before searching.
    expect(wrapper.findAll(".group-head").length).toBe(2);

    const input = wrapper.find('input[type="search"]');
    await input.setValue("tokio");
    await flushPromises();

    // No grouped headers while searching.
    expect(wrapper.findAll(".group-head").length).toBe(0);
    // One flat result.
    const rows = wrapper.findAll(".pkg-row");
    expect(rows.length).toBe(1);
    expect(rows[0].text()).toContain("tokio");

    // Clearing the query drops back to the grouped view.
    await input.setValue("");
    await flushPromises();
    expect(wrapper.findAll(".group-head").length).toBe(2);
  });

  it("shows the no-results message for an unmatched query", async () => {
    const { wrapper } = mountWithApp(LicensesSection);
    await flushPromises();
    await wrapper.find('input[type="search"]').setValue("does-not-exist");
    await flushPromises();
    expect(wrapper.findAll(".pkg-row").length).toBe(0);
    expect(wrapper.text()).toContain("does-not-exist");
  });

  it("shows the degraded notice when the inventory is incomplete", async () => {
    __resetLicensesCacheForTests();
    mockFetchOk({ ...sample, complete: false, note: "incomplete" });
    const { wrapper } = mountWithApp(LicensesSection);
    await flushPromises();
    expect(wrapper.text()).toContain("incomplete");
  });

  it("shows the empty alert when the inventory is complete but has 0 packages", async () => {
    __resetLicensesCacheForTests();
    mockFetchOk({ ...sample, packages: [], ecosystems: {} });
    const { wrapper } = mountWithApp(LicensesSection);
    await flushPromises();
    expect(wrapper.text()).toContain("No license data available.");
    // Not the failure alert, and no summary.
    expect(wrapper.text()).not.toContain("Couldn't load");
    expect(wrapper.find(".summary").exists()).toBe(false);
  });

  it("shows the load-failed alert when the fetch fails (degraded doc)", async () => {
    __resetLicensesCacheForTests();
    // HTTP failure → fetchLicenses resolves to a degraded doc (0 packages,
    // complete:false) → the `failed` computed renders the loadFailed alert.
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({ ok: false, status: 500 }),
    );
    const { wrapper } = mountWithApp(LicensesSection);
    await flushPromises();
    expect(wrapper.text()).toContain("Couldn't load the license inventory.");
    expect(wrapper.find(".summary").exists()).toBe(false);
  });

  it("falls back to the no-license-text message for a package without text", async () => {
    __resetLicensesCacheForTests();
    // serde with empty licenseText (the degraded/Cargo.lock-fallback shape).
    const noText = {
      ...sample,
      packages: sample.packages.map((p) =>
        p.name === "serde" ? { ...p, licenseText: "" } : p,
      ),
    };
    mockFetchOk(noText);
    const { wrapper } = mountWithApp(LicensesSection);
    await flushPromises();
    await wrapper.findAll(".group-head")[0].trigger("click");
    const serdeRow = wrapper
      .findAll(".pkg-row")
      .find((r) => r.text().includes("serde"))!;
    await serdeRow.trigger("click");
    expect(wrapper.text()).toContain(
      "License text unavailable for this package.",
    );
  });
});
