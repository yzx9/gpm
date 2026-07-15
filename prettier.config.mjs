// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { createRequire } from "node:module";

const localRequire = createRequire(import.meta.url);

// prettier-plugin-organize-imports is intentionally optional. When the
// dependency is installed (CI, or local dev after `pnpm install`) it sorts and
// merges imports; in a fresh worktree without node_modules it is skipped so
// `prettier` / `just fmt` never fail on a missing module. CI's `prettier
// --check` is the authoritative gate, so skipping locally cannot let import
// drift through unnoticed.
const plugins = [];
try {
  localRequire.resolve("prettier-plugin-organize-imports");
  plugins.push("prettier-plugin-organize-imports");
} catch {
  // Plugin not installed — prettier runs without import organization.
}

/** @type {import("prettier").Config} */
export default {
  plugins,
};
