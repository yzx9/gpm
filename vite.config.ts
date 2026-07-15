// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import VueI18nPlugin from "@intlify/unplugin-vue-i18n/vite";
import tailwindcss from "@tailwindcss/vite";
import vue from "@vitejs/plugin-vue";
import { defineConfig } from "vite";
// @ts-expect-error node:url is a nodejs module (this project ships no @types/node)
import { fileURLToPath, URL } from "node:url";
// @ts-expect-error local .mjs plugin (ships no type declarations)
import { licensesPlugin } from "./scripts/vite-plugin-licenses.mjs";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// Vite config root — used to build an absolute glob for the i18n plugin below.
const root = fileURLToPath(new URL("./", import.meta.url));

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [
    vue(),
    tailwindcss(),
    // Regenerate the open-source license inventory (public/licenses.json) at
    // dev/build start. Staleness-aware; failures are swallowed (the Licenses
    // tab renders a degraded-state notice when the file is missing/empty).
    licensesPlugin(),
    // Precompile every locale JSON bundle at build time so the runtime message
    // compiler doesn't ship to the WebView. (`legacy: false` is set on
    // `createI18n` itself, not here.)
    VueI18nPlugin({
      include: [`${root}src/locales/**/*.json`],
    }),
  ],
  resolve: { alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) } },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
