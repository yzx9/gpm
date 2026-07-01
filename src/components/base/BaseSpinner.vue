<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
withDefaults(
  defineProps<{
    /** "white" for use on accent/dark backgrounds, "dark" (default) elsewhere. */
    variant?: "dark" | "white";
  }>(),
  { variant: "dark" },
);
</script>

<template>
  <span
    class="spinner"
    :class="{ 'spinner-white': variant === 'white' }"
    aria-hidden="true"
  ></span>
</template>

<style scoped>
/* Uses the global @keyframes spin from src/assets/main.css. The spinner itself
   carries no margin — spacing comes from the parent's flex gap (e.g. BaseButton)
   or a Tailwind gap wrapper for standalone spinner + text. */
.spinner {
  display: inline-block;
  width: 14px;
  height: 14px;
  border: 2px solid var(--color-edge);
  border-top-color: var(--color-accent);
  border-radius: 50%;
  animation: spin 0.6s linear infinite;
  vertical-align: middle;
}

.spinner-white {
  border-color: rgba(255, 255, 255, 0.3);
  border-top-color: white;
}
</style>
