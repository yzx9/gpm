// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

// Generates `public/licenses.json` — the complete open-source license
// inventory the "About → Licenses" page renders.
//
// Coverage is the full transitive tree, not just direct dependencies: every
// crate/package compiled or bundled into gpm ships its code with the binary,
// so MIT/Apache/BSD obligations attach to each one. The list is large by
// design (~hundreds; the Rust tree via `git2`/`age`/`rpgp`/`tokio`/`serde` is
// the bulk) and the UI absorbs the scale with "group by license + fold +
// search" — see `LicensesSection.vue`.
//
// Sources:
//  - Rust crates:  `cargo metadata --format-version 1` (always available with
//                  a cargo toolchain; no extra install). Workspace members are
//                  gpm's own crates and are excluded (acknowledged separately).
//  - npm packages: the runtime transitive closure of `dependencies`, resolved
//                  via Node's own module resolver (createRequire) so pnpm's
//                  symlink layout is honored. Dev tooling (vite/vitest/
//                  tailwind) is excluded — it never ships in the Tauri bundle.
//
// Not yet covered: Android Gradle dependencies (androidx, Kotlin stdlib, …)
// ship in the APK but aren't scanned here (needs a gradle dependency walk).
// The omission is invisible in the UI — the `complete` flag only tracks cargo
// presence — so surface this as a follow-up if a gradle-side (e.g. Apache-2.0)
// obligation ever needs listing.
//
// Degradation: if `cargo` is not on PATH (e.g. a frontend-only shell without
// the Rust toolchain), `complete` is `false` and Rust crates are listed from
// `Cargo.lock` with names+versions only (no license text). The page surfaces a
// "regenerate with the full toolchain" notice instead of failing.

import { execFileSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));

/** Filenames (case-insensitive) that carry license text. Matches the stem
 *  (`LICENSE`/`COPYING`/`UNLICENSE`) plus suffixed forms like `LICENSE-MIT` /
 *  `LICENSE-APACHE` (what `cargo new` generates for `MIT OR Apache-2.0`, the
 *  bulk of the Rust tree) and prefixed forms like `MIT-LICENSE` /
 *  `APACHE-LICENSE`. NOTICE/COPYRIGHT are excluded — attribution, not license
 *  text. Backup/temp files that share the stem are filtered by isBackupFile. */
const LICENSE_FILE_RE =
  /^(?:licen[cs]e|copying|unlicense|(?:mit|apache|bsd|mpl)[-_]?licen[cs]e)(?:[-_.][a-z0-9]+)*$/i;

/** True for backup/temp files that coincidentally match the license stem
 *  (LICENSE.bak, LICENSE~) and would pollute the license text. */
function isBackupFile(name) {
  return /\.(bak|tmp|orig|swp|save|old)$/i.test(name) || name.endsWith("~");
}

/**
 * @typedef {Object} LicensePackage
 * @property {"rust"|"npm"} ecosystem
 * @property {string} name
 * @property {string} version
 * @property {string} license  SPDX expression or "UNKNOWN"
 * @property {string} repository
 * @property {string} licenseText  Full license text ("" if unavailable)
 *
 * @typedef {Object} LicensesData
 * @property {string|null} generatedAt  ISO timestamp (null if undeterministic)
 * @property {boolean} complete  false if any source was skipped/degraded
 * @property {string} note  human-readable caveat, "" when fully complete
 * @property {Record<string, number>} ecosystems  per-ecosystem package counts
 * @property {LicensePackage[]} packages
 */

/** Find and read the license text in `dir`. All matching license files are
 *  concatenated (sorted) so a dual-licensed crate that ships LICENSE-APACHE +
 *  LICENSE-MIT shows BOTH texts rather than an arbitrary one. "" if none. */
function readLicenseText(dir) {
  let entries;
  try {
    entries = readdirSync(dir);
  } catch {
    return "";
  }
  const matches = entries
    .filter((n) => LICENSE_FILE_RE.test(n) && !isBackupFile(n))
    .sort();
  if (matches.length === 0) return "";
  return matches
    .map((n) => {
      try {
        return readFileSync(join(dir, n), "utf8");
      } catch {
        return "";
      }
    })
    .filter((t) => t.length > 0)
    .join("\n\n");
}

/** Read a specific designated license file (the `license-file` manifest field),
 *  resolved relative to `dir`. "" on any failure. */
function readDesignatedLicenseFile(dir, rel) {
  if (!rel) return "";
  try {
    return readFileSync(join(dir, rel), "utf8");
  } catch {
    return "";
  }
}

/** Normalize an SPDX-ish license string; "" / null → "UNKNOWN". */
function normalizeLicense(raw) {
  if (!raw || typeof raw !== "string") return "UNKNOWN";
  const t = raw.trim();
  return t.length ? t : "UNKNOWN";
}

/**
 * Run `cargo metadata` and map every non-workspace package to a LicensePackage.
 * Returns `{ packages, ok }`; `ok` is false when cargo is unavailable/errors.
 */
function scanRust(root) {
  let meta;
  try {
    // `cargo metadata` lists every resolved crate (the full transitive tree).
    // `--no-deps` is intentionally omitted — we WANT dependencies.
    const out = execFileSync("cargo", ["metadata", "--format-version", "1"], {
      cwd: root,
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
      stdio: ["ignore", "pipe", "ignore"],
    });
    meta = JSON.parse(out);
  } catch {
    return { packages: [], ok: false };
  }
  const workspaceIds = new Set(meta.workspace_members ?? []);
  const packages = [];
  for (const p of meta.packages ?? []) {
    if (workspaceIds.has(p.id)) continue; // gpm's own crates
    const dir = p.manifest_path ? dirname(p.manifest_path) : "";
    // Honor the crate's designated `license-file` (cargo-about parity) when it
    // omits an SPDX `license`; otherwise scan the directory for LICENSE* files.
    const licenseText = dir
      ? readDesignatedLicenseFile(dir, p.license_file) || readLicenseText(dir)
      : "";
    packages.push({
      ecosystem: "rust",
      name: p.name ?? "UNKNOWN",
      version: p.version ?? "",
      license: normalizeLicense(p.license),
      repository: p.repository ?? "",
      licenseText,
    });
  }
  return { packages, ok: true };
}

/** Parse Cargo.lock as a degraded fallback (names+versions, no license text). */
function scanRustFromLock(root) {
  const lockPath = join(root, "Cargo.lock");
  if (!existsSync(lockPath)) return [];
  const text = readFileSync(lockPath, "utf8");
  const pkgs = [];
  let name = "";
  let version = "";
  for (const line of text.split("\n")) {
    const m = line.match(/^name\s*=\s*"(.+)"$/);
    if (m) name = m[1];
    const v = line.match(/^version\s*=\s*"(.+)"$/);
    if (v) version = v[1];
    if (line.startsWith("source =") && name && version) {
      // A `source` line marks a crate from a registry (i.e. NOT a workspace
      // member, which has no source). Capture it.
      pkgs.push({
        ecosystem: "rust",
        name,
        version,
        license: "UNKNOWN",
        repository: "",
        licenseText: "",
      });
      name = "";
      version = "";
    }
  }
  return pkgs;
}

// Policy note — why Rust and npm are scoped differently:
//  - Rust:  the binary links across the *full* resolved tree (and build/proc-
//           macro crates are part of the resolved Cargo.lock), so we list every
//           crate `cargo metadata` reports — matching `cargo about` and the
//           "About → Open-source licenses" convention (VS Code, Android's
//           oss-licenses plugin).
//  - npm:   only the runtime transitive closure of `dependencies` ships in the
//           Tauri bundle. Dev tooling (vite, vitest, tailwind, prettier) never
//           reaches users, so we exclude `devDependencies` rather than claim
//           credit for tools we don't distribute.

/** Read a package.json as JSON, or null on any failure. */
function readPackageJson(path) {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch {
    return null;
  }
}

/** Build a LicensePackage from a parsed package.json + its directory. */
function npmEntry(pj, dir) {
  let license = normalizeLicense(pj.license);
  if (license === "UNKNOWN" && Array.isArray(pj.licenses)) {
    // Legacy `licenses: [{type, url} | {name, url}]` shape — pick the SPDX id
    // from `type` (or `name`), skipping entries that expose neither so we never
    // join a raw object into "[object Object]".
    const ids = pj.licenses
      .map((l) => (typeof l === "string" ? l : (l.type ?? l.name)))
      .filter((s) => typeof s === "string" && s.length > 0);
    license = normalizeLicense(ids.join(" OR "));
  }
  return {
    ecosystem: "npm",
    name: pj.name,
    version: pj.version,
    license,
    repository:
      typeof pj.repository === "string"
        ? pj.repository
        : (pj.repository?.url ?? ""),
    licenseText: readLicenseText(dir),
  };
}

/** Resolve `<name>/package.json` reachable from `fromDir` (a package directory),
 *  honoring pnpm's symlink layout via Node's own resolver. Returns the path or
 *  null if the package isn't installed/ resolvable. */
function resolvePkgJsonPath(name, fromDir) {
  try {
    return createRequire(join(fromDir, "package.json")).resolve(
      `${name}/package.json`,
    );
  } catch {
    return null;
  }
}

/** Walk the runtime transitive closure of the root `dependencies`, starting
 *  from the repo root. Dev tooling is excluded (see policy note above). */
function scanNpm(root) {
  const rootPj = readPackageJson(join(root, "package.json"));
  if (!rootPj) return [];
  const seen = new Set();
  const packages = [];
  // BFS queue of [depName, fromDir]. Root deps resolve from the repo root.
  const queue = Object.keys(rootPj.dependencies ?? {}).map((name) => [
    name,
    root,
  ]);
  while (queue.length) {
    const [name, fromDir] = queue.shift();
    const pjPath = resolvePkgJsonPath(name, fromDir);
    if (!pjPath) continue;
    const dir = dirname(pjPath);
    const pj = readPackageJson(pjPath);
    if (!pj || !pj.name || !pj.version) continue;
    const key = `npm:${pj.name}@${pj.version}`;
    if (seen.has(key)) continue;
    seen.add(key);
    packages.push(npmEntry(pj, dir));
    // Enqueue this package's own runtime dependencies, resolved from ITS
    // directory so pnpm's per-package node_modules is used.
    for (const child of Object.keys(pj.dependencies ?? {})) {
      queue.push([child, dir]);
    }
  }
  return packages;
}

/**
 * Generate the license inventory and write it to `outPath`.
 *
 * @param {Object} [opts]
 * @param {string} [opts.root]   Repo root (defaults to the parent of scripts/).
 * @param {string} [opts.out]    Output path (default <root>/public/licenses.json).
 * @param {boolean} [opts.force] Write even when the output looks fresh.
 * @returns {{ wrote: boolean, path: string, data: LicensesData }}
 */
export function generateLicenses(opts = {}) {
  const root = resolve(opts.root ?? join(__dirname, ".."));
  const out = resolve(opts.out ?? join(root, "public", "licenses.json"));
  // Skip work when fresh unless forced: the inventory is a pure function of
  // Cargo.lock + package.json + the installed node_modules, so regenerate only
  // when any of those is newer than the output. (pnpm-lock.yaml is included so
  // a transitive-only bump — which may not move the top-level node_modules dir
  // mtime — still invalidates the cache.)
  if (!opts.force && existsSync(out)) {
    const outMtime = statSync(out).mtimeMs;
    const sources = [
      "Cargo.lock",
      "package.json",
      "pnpm-lock.yaml",
      "node_modules",
    ];
    const newest = Math.max(
      0,
      ...sources.map((s) =>
        existsSync(join(root, s)) ? statSync(join(root, s)).mtimeMs : 0,
      ),
    );
    if (outMtime >= newest) {
      let cached;
      try {
        cached = JSON.parse(readFileSync(out, "utf8"));
      } catch {
        cached = null;
      }
      // A cached degraded doc (complete === false, e.g. cargo was missing last
      // run) is NOT honored: fall through and regenerate so installing the
      // toolchain (or any earlier failure recovering) is picked up next run.
      if (cached && cached.complete !== false) {
        return { wrote: false, path: out, data: cached };
      }
    }
  }

  const rust = scanRust(root);
  const rustPackages = rust.ok ? rust.packages : scanRustFromLock(root);
  const npmPackages = scanNpm(root);

  const packages = [...rustPackages, ...npmPackages].sort((a, b) => {
    if (a.ecosystem !== b.ecosystem) return a.ecosystem < b.ecosystem ? -1 : 1;
    if (a.name !== b.name) return a.name < b.name ? -1 : 1;
    return a.version < b.version ? -1 : a.version > b.version ? 1 : 0;
  });

  const ecosystems = packages.reduce((acc, p) => {
    acc[p.ecosystem] = (acc[p.ecosystem] ?? 0) + 1;
    return acc;
  }, {});

  const notes = [];
  if (!rust.ok) {
    notes.push(
      "Rust licenses were generated from Cargo.lock without the cargo toolchain — license text is missing. Re-run with cargo on PATH for the full inventory.",
    );
  }

  /** @type {LicensesData} */
  const data = {
    // Deterministic null: a real timestamp would make the artifact non-
    // reproducible across builds and churn the gitignored file needlessly.
    generatedAt: null,
    complete: notes.length === 0,
    note: notes.join(" "),
    ecosystems,
    packages,
  };

  // Write synchronously so the vite buildStart hook can await before serve.
  mkdirSync(dirname(out), { recursive: true });
  writeFileSync(out, JSON.stringify(data), "utf8");
  return { wrote: true, path: out, data };
}

// CLI entry: `node scripts/gen-licenses.mjs [--force]`
const isMain = (() => {
  try {
    return (
      process.argv[1] &&
      resolve(process.argv[1]) === fileURLToPath(import.meta.url)
    );
  } catch {
    return false;
  }
})();
if (isMain) {
  const force = process.argv.includes("--force");
  const r = generateLicenses({ force });
  const n = r.data.packages.length;
  const eco = Object.entries(r.data.ecosystems)
    .map(([k, v]) => `${k}=${v}`)
    .join(" ");
  console.log(
    `licenses: ${r.wrote ? "wrote" : "cached"} ${r.path} (${n} packages; ${eco}${r.data.complete ? "" : "; INCOMPLETE"})}`,
  );
}
