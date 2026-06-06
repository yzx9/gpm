<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

const configured = ref(false);

onMounted(async () => {
  try {
    configured.value = await invoke<boolean>("is_configured");
  } catch {
    configured.value = false;
  }
});
</script>

<template>
  <router-view />
</template>

<style>
/* ---------------------------------------------------------------------------
 * gpm Design System
 * Minimal tokens for a 3-page trust-focused password client.
 * Dark theme primary, single accent, monospace for crypto content.
 * --------------------------------------------------------------------------- */
:root {
  /* Typography */
  --font-sans:
    -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue",
    Arial, sans-serif;
  --font-mono:
    "SF Mono", "JetBrains Mono", "Fira Code", "Cascadia Code", monospace;
  --font-size-xs: 0.75rem;
  --font-size-sm: 0.85rem;
  --font-size-base: 0.9rem;
  --font-size-md: 1rem;
  --font-size-lg: 1.1rem;
  --font-size-xl: 1.25rem;
  --font-size-display: 1.75rem;
  --font-weight-normal: 400;
  --font-weight-medium: 500;
  --font-weight-semibold: 600;

  /* Colors — Light */
  --bg-primary: #f8f9fa;
  --bg-surface: #ffffff;
  --bg-input: #fafafa;
  --bg-hover: #f0f0f5;
  --text-primary: #1a1a2e;
  --text-secondary: #888888;
  --text-tertiary: #aaaaaa;
  --border: #dddddd;
  --accent: #4a6cf7;
  --accent-hover: #3a5ce5;
  --accent-focus-ring: rgba(74, 108, 247, 0.2);
  --danger: #c33;
  --danger-bg: #fee;
  --danger-border: #fcc;
  --success: #2e7d32;
  --success-bg: #e8f5e9;
  --info: #1976d2;
  --info-bg: #e8f4fd;

  /* Spacing */
  --space-xs: 0.25rem;
  --space-sm: 0.5rem;
  --space-md: 0.75rem;
  --space-lg: 1rem;
  --space-xl: 1.5rem;
  --space-2xl: 2rem;

  /* Layout */
  --max-width: 480px;
  --screen-padding: 1rem;
  --radius-sm: 6px;
  --radius-md: 8px;
  --radius-lg: 12px;

  /* Touch */
  --touch-min: 48px;

  /* Apply base styles */
  font-family: var(--font-sans);
  font-size: 16px;
  line-height: 1.5;
  color: var(--text-primary);
  background-color: var(--bg-primary);
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
}

@media (prefers-color-scheme: dark) {
  :root {
    --bg-primary: #0d1117;
    --bg-surface: #161b22;
    --bg-input: #1a1a2e;
    --bg-hover: #2f2f50;
    --text-primary: #e6edf3;
    --text-secondary: #8b949e;
    --text-tertiary: #6e7681;
    --border: #444;
    --danger-bg: #3a1a1a;
    --danger-border: #5a2a2a;
    --success-bg: #1a3a1a;
    --info-bg: #1a2a3a;
  }
}

* {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

body {
  min-height: 100vh;
}
</style>
