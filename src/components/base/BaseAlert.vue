<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { computed } from "vue";

const props = defineProps<{
  variant: "danger" | "success" | "info" | "warning";
  /** Defaults to "alert" for danger, "status" elsewhere (with aria-live="polite").
   *  Override for danger-colored-but-informational notices (e.g. biometric reset). */
  role?: string;
}>();

const resolvedRole = computed(
  () => props.role ?? (props.variant === "danger" ? "alert" : "status"),
);
const ariaLive = computed(() =>
  resolvedRole.value === "status" ? "polite" : undefined,
);

// Literal class strings so Tailwind's scanner generates the utilities.
const VARIANT: Record<typeof props.variant, string> = {
  danger: "bg-danger-soft text-danger",
  success: "bg-success-soft text-success",
  info: "bg-info-soft text-info",
  warning: "bg-warning-soft text-warning",
};
</script>

<template>
  <div
    class="p-2 px-3 rounded-sm text-sm"
    :class="VARIANT[variant]"
    :role="resolvedRole"
    :aria-live="ariaLive"
  >
    <slot />
  </div>
</template>
