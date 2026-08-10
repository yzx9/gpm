// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { beforeAll, describe, expect, it } from "vitest";
// @ts-expect-error node:path is a nodejs module (project ships no @types/node)
import { resolve } from "node:path";
// @ts-expect-error local .mjs build script ships no type declarations
import { scanNpmAll } from "../../../../scripts/gen-licenses.mjs";

// scanNpmAll covers BOTH workspace packages: the frontend app (app/, where
// vitest runs via `pnpm -C app test`) and the workspace root (where the
// formatting toolchain — prettier + prettier-plugin-organize-imports — lives as
// devDeps). Needs a real node_modules — `pnpm install` first in a fresh worktree
// (the whole FE suite already requires it).
// @ts-expect-error process is a nodejs global (project ships no @types/node)
const appDir = process.cwd();
const root = resolve(appDir, ".."); // workspace root

// A few-hundred-package BFS; run it once for the whole file. The import is
// untyped (local .mjs), so cast the entry shape we rely on.
let pkgs: { name: string; version: string }[] = [];
beforeAll(() => {
  pkgs = scanNpmAll(root, appDir) as { name: string; version: string }[];
});

describe("gen-licenses scanNpm", () => {
  it("attributes dev tooling from both the workspace root and the app", () => {
    const names = pkgs.map((p) => p.name);
    // scanNpmAll seeds from BOTH dependencies and devDependencies across both
    // workspace packages, so build/test tooling is attributed despite not
    // shipping. prettier lives at the workspace ROOT (not under app/) — a
    // peerDep of prettier-plugin-organize-imports that the BFS never walks, so
    // it must be seeded as a direct root devDep; scanning the root keeps it in
    // the inventory.
    for (const dep of [
      "vite",
      "vitest",
      "jsdom",
      "typescript",
      "vue-tsc",
      "tailwindcss",
      "prettier",
    ])
      expect(names).toContain(dep);
    // Runtime deps still present (regression guard on the existing behavior):
    for (const dep of ["vue", "@tauri-apps/api"]) expect(names).toContain(dep);
    // Dev tree expanded well past the runtime-only ~30 baseline:
    expect(pkgs.length).toBeGreaterThan(50); // bump if toolchain shrinks
  });

  it("walks a dev-seeded package's own runtime dependencies", () => {
    // vite is seeded as a root devDep; its runtime deps must be pulled in by the
    // `pj.dependencies` recursion — the core of the change, not just the seed.
    const names = pkgs.map((p) => p.name);
    expect(names).toContain("rolldown");
    expect(names).toContain("postcss");
  });

  it("dedupes by name@version (never lists the same name@version twice)", () => {
    // The `npm:name@version` seen-set collapses identical pairs reached via
    // different paths. Multiple versions of the same name are expected (each
    // carries its own license text) — only exact name@version dupes are bugs.
    const keys = pkgs.map((p) => `${p.name}@${p.version}`);
    expect(new Set(keys).size).toBe(keys.length);
  });

  it("resolves packages whose exports map blocks the /package.json subpath", () => {
    // Canary for resolvePkgJsonPath's exports-restriction fallback: each of
    // these ships a restrictive `exports` field that does NOT expose
    // './package.json', so the fast-path resolve throws and the bare-name +
    // dirname-walk fallback must recover them. Two are ROOT RUNTIME deps, so
    // this also guards real attribution coverage, not just the count.
    const names = pkgs.map((p) => p.name);
    for (const dep of [
      "@tauri-apps/plugin-clipboard-manager",
      "@tauri-apps/plugin-opener",
      "@vitejs/plugin-vue",
    ])
      expect(names).toContain(dep);
  });
});
