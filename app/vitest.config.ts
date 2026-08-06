// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import VueI18nPlugin from "@intlify/unplugin-vue-i18n/vite";
import vue from "@vitejs/plugin-vue";
import { defineConfig } from "vitest/config";
// @ts-expect-error node:url is a nodejs module (this project ships no @types/node)
import { fileURLToPath, URL } from "node:url";

// Config root — used to build an absolute glob for the i18n plugin below.
const root = fileURLToPath(new URL("./", import.meta.url));

export default defineConfig({
  plugins: [
    vue(),
    // Mirror the production `vite build`: precompile every locale JSON bundle
    // so tests exercise the same compiled-message shapes the WebView ships.
    // Without this the test env diverges from production — e.g. `tm` returns the
    // raw string in tests but a compiled AST object in the real build — and
    // IPC-contract regressions (a non-string slipping into a command arg) hide
    // behind a green suite.
    VueI18nPlugin({ include: [`${root}src/locales/**/*.json`] }),
  ],
  resolve: { alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) } },

  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
    globals: true,
  },
});
