<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
withDefaults(
  defineProps<{
    /** `flat` = bordered settings/modal card; `raised` = borderless shadowed gate card. */
    variant?: "flat" | "raised";
    /** Polymorphic root tag (e.g. "section" for settings sections). */
    as?: string;
    /** Flat-card border tone: `danger` (Danger Zone), `accent` (pending/unsaved). */
    border?: "edge" | "danger" | "accent";
  }>(),
  { variant: "flat", as: "div", border: "edge" },
);
</script>

<template>
  <component
    :is="as"
    class="card"
    :class="[
      variant,
      {
        'danger-border': border === 'danger',
        'accent-border': border === 'accent',
      },
    ]"
  >
    <slot />
  </component>
</template>

<style scoped>
.card {
  background: var(--color-surface);
}
/* Bordered settings/modal card (formerly .settings-card / .modal-card). */
.flat {
  padding: 1rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
}
/* Borderless shadowed card (formerly the UnlockModal/AppLockOverlay .card). */
.raised {
  padding: 2rem;
  border-radius: var(--radius-lg);
  box-shadow: 0 2px 12px rgba(0, 0, 0, 0.08);
}
.flat.danger-border {
  border-color: var(--color-danger-edge, var(--color-danger, #c66));
}
/* Pending/unsaved-changes tone — mirrors the accent focus ring on inputs. */
.flat.accent-border {
  border-color: var(--color-accent);
  box-shadow: 0 0 0 2px var(--color-accent-ring);
}
</style>
