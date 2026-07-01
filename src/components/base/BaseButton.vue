<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { computed } from "vue";
import BaseSpinner from "./BaseSpinner.vue";

const props = withDefaults(
  defineProps<{
    /** Visual style. `action` / `action-danger` are full-width, left-aligned. */
    variant?:
      | "primary"
      | "secondary"
      | "outline"
      | "danger"
      | "action"
      | "action-danger";
    /** `md` (default) or `sm`. Ignored for `action` variants (fixed compact size). */
    size?: "md" | "sm";
    /** Shows a leading spinner (white on `primary`, dark elsewhere) and disables. */
    loading?: boolean;
    /** Stretch to 100% width. */
    block?: boolean;
    /** Native button type. */
    type?: "button" | "submit" | "reset";
    /** Forwarded to the native button; also forced on when `loading`. */
    disabled?: boolean;
  }>(),
  {
    variant: "secondary",
    size: "md",
    loading: false,
    block: false,
    type: "button",
    disabled: false,
  },
);

const sizeClass = computed(() =>
  props.variant === "action" || props.variant === "action-danger"
    ? null
    : `size-${props.size}`,
);
</script>

<template>
  <button
    :type="type"
    class="btn"
    :class="[variant, sizeClass, { block: block }]"
    :disabled="disabled || loading"
  >
    <BaseSpinner
      v-if="loading"
      :variant="variant === 'primary' ? 'white' : 'dark'"
    />
    <slot />
  </button>
</template>

<style scoped>
/* Base layout. Original buttons were a mix of inline-block and flex; unifying on
   inline-flex + gap lets a leading spinner or icon align without per-variant
   tweaks. Visuals are preserved: auto-width buttons shrink to content, full-width
   variants set width:100% themselves. */
.btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 0.4rem;
  font-family: inherit;
  font-weight: 500;
  cursor: pointer;
  transition: background 0.2s;
  min-height: 48px;
}
.btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.block {
  width: 100%;
}

/* Sizes */
.size-md {
  padding: 0.75rem;
  font-size: var(--text-md);
  border-radius: var(--radius-md);
}
.size-sm {
  padding: 0.5rem 0.75rem;
  font-size: var(--text-sm);
  border-radius: var(--radius-sm);
}

/* Variants */
.primary {
  background: var(--color-accent);
  color: white;
  border: none;
}
.primary:hover:not(:disabled) {
  background: var(--color-accent-deep);
}

.secondary {
  background: var(--color-surface);
  color: inherit;
  border: 1px solid var(--color-edge);
}
.secondary:hover:not(:disabled) {
  background: var(--color-hover);
}

/* Accent-outlined secondary (e.g. entry-detail actions). */
.outline {
  background: var(--color-surface);
  color: var(--color-accent);
  border: 1px solid var(--color-accent);
}
.outline:hover:not(:disabled) {
  background: var(--color-hover);
}

.danger {
  background: var(--color-surface);
  color: var(--color-danger);
  border: 1px solid var(--color-danger);
}
.danger:hover:not(:disabled) {
  background: var(--color-danger-soft);
}

/* Action variants: full-width, left-aligned, compact (size prop ignored). */
.action,
.action-danger {
  width: 100%;
  justify-content: flex-start;
  text-align: left;
  padding: 0.5rem 0.75rem;
  font-size: var(--text-sm);
  border-radius: var(--radius-md);
  background: var(--color-surface);
  border: 1px solid var(--color-edge);
}
.action:hover:not(:disabled) {
  background: var(--color-hover);
}
.action-danger {
  border-color: var(--color-danger-edge, var(--color-danger, #c66));
  color: #c66;
}
.action-danger:hover:not(:disabled) {
  background: var(--color-hover);
}
</style>
