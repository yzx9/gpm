<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<script setup lang="ts">
withDefaults(
  defineProps<{
    /** `flat` = bordered settings/modal card; `raised` = borderless shadowed gate card. */
    variant?: "flat" | "raised";
    /** Polymorphic root tag (e.g. "section" for settings sections). */
    as?: string;
    /** Flat-card border tone: `danger` (Danger Zone), `accent` (pending/unsaved). */
    border?: "edge" | "danger" | "accent";
    /** One-shot accent ring that lights up then fades (~1.6s, no `forwards` fill
     *  so the card returns to normal) — e.g. a deep-link target from the
     *  Permissions screen. */
    highlight?: boolean;
  }>(),
  { variant: "flat", as: "div", border: "edge", highlight: false },
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
        'card-highlight': highlight,
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
/* One-shot highlight ring for a card you've been deep-linked to (e.g. the
   biometric/passphrase card from the Permissions screen): a solid accent ring
   lights up, then fades out. No `forwards` fill, so the card returns to normal.
   Vue scopes the keyframe name to this component. */
@keyframes card-highlight-ring {
  0% {
    box-shadow: 0 0 0 0 transparent;
  }
  30% {
    box-shadow: 0 0 0 4px var(--color-accent);
  }
  100% {
    box-shadow: 0 0 0 4px transparent;
  }
}
.card-highlight {
  animation: card-highlight-ring 1.6s ease-out;
}
</style>
