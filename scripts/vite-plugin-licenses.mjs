// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

// Vite plugin: ensures `public/licenses.json` exists (and is fresh) before the
// dev server serves or the production bundle is written. Runs once per server
// start / build — `buildStart` fires for both. The generator is staleness-aware
// (it no-ops when Cargo.lock / package.json / node_modules haven't moved), so
// this adds at most one `cargo metadata` call per cold start and nothing on HMR
// reloads.
//
// Failure policy:
//  - serve (dev): warn and keep going — a missing file just shows the Licenses
//    tab's degraded notice, and the next server start retries.
//  - build (release): rethrow on a generator EXCEPTION (e.g. can't write the
//    file). Note this does NOT fire when cargo is absent: generateLicenses
//    swallows a cargo failure and writes a degraded (complete:false) doc, which
//    ships with the tab's "incomplete" notice. That's academic in practice — a
//    real `tauri build` compiles the Rust backend, so cargo metadata succeeds.

import { generateLicenses } from "./gen-licenses.mjs";

/** @type {'serve' | 'build' | null} */
let command = null;
let ran = false;

export function licensesPlugin() {
  return {
    name: "gpm:gen-licenses",
    config(_, { command: cmd }) {
      command = cmd;
    },
    async buildStart() {
      // `buildStart` also fires per sub-build in some setups; guard so a single
      // dev server start generates once. Set only after success so a transient
      // failure (e.g. disk-full on write) retries on the next start.
      if (ran) return;
      try {
        generateLicenses();
        ran = true;
      } catch (e) {
        const msg = `licenses.json generation failed: ${e?.message ?? e}`;
        if (command === "build") throw new Error(msg);
        this.warn?.(msg);
      }
    },
  };
}
