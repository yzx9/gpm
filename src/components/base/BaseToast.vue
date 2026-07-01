<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
defineProps<{
  variant: "danger" | "success" | "info" | "warning";
}>();

// Literal class strings so Tailwind's scanner generates the utilities.
const VARIANT = {
  danger: "bg-danger-soft text-danger",
  success: "bg-success-soft text-success",
  info: "bg-info-soft text-info",
  warning: "bg-warning-soft text-warning",
} as const;
</script>

<template>
  <div
    class="fixed bottom-4 left-1/2 -translate-x-1/2 p-2 px-4 rounded-md text-sm shadow-lg z-50"
    :class="VARIANT[variant]"
    role="status"
    aria-live="polite"
  >
    <slot />
  </div>
</template>
