// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { describe, expect, it } from "vitest";
import { routes } from "./routes";

// Every en locale bundle that ships with the app (keys are the glob's
// repo-relative module paths).
const enBundles = import.meta.glob("./locales/en/*.json");

// The router guard resolves each route's i18n namespace as
// `meta.bundle ?? route.name` and loadBundle silently swallows a missing
// bundle — so a route whose effective namespace has no JSON renders raw keys
// (e.g. `about.licensesTitle`) on a cold deep-link, and nothing else fails:
// page tests inline every bundle and never exercise the real route table.
// This pins the wiring for every route, present and future.
describe("route i18n wiring", () => {
  it("each named route's namespace (meta.bundle ?? name) ships an en bundle", () => {
    for (const r of routes) {
      if (typeof r.name !== "string") continue; // redirect-only records
      const ns = (r.meta?.bundle as string | undefined) ?? r.name;
      expect(
        enBundles,
        `route "${r.name}" would load non-existent bundle "${ns}"`,
      ).toHaveProperty(`./locales/en/${ns}.json`);
    }
  });
});
