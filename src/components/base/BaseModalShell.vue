<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { computed } from "vue";
import BaseCard from "./BaseCard.vue";

const props = withDefaults(
  defineProps<{
    /** `center` = always-centered gate (unlock/app-lock); `sheet` = bottom-sheet on mobile, centered ≥sm. */
    variant: "center" | "sheet";
    /** Stacking layer. Defaults: center→60, sheet→40. AppLockOverlay passes 70 to sit above the identity modal. */
    z?: number;
    /** Accessibility role; `dialog` (default) or `alertdialog`. */
    role?: string;
  }>(),
  { role: "dialog" },
);

const emit = defineEmits<{ (e: "close"): void }>();

const resolvedZ = computed(
  () => props.z ?? (props.variant === "center" ? 60 : 40),
);

// Backdrop click closes only when the caller listens for `@close` (no-op
// otherwise), preserving sites that intentionally dismiss only via buttons.
function onBackdrop() {
  emit("close");
}
</script>

<template>
  <div
    class="overlay"
    :class="variant"
    :style="{ zIndex: resolvedZ }"
    :role="role"
    aria-modal="true"
    @click.self="onBackdrop"
  >
    <div class="wrap" :class="variant">
      <BaseCard :variant="variant === 'center' ? 'raised' : 'flat'">
        <slot />
      </BaseCard>
    </div>
  </div>
</template>

<style scoped>
.overlay {
  position: fixed;
  inset: 0;
  display: flex;
  justify-content: center;
  background: rgba(0, 0, 0, 0.4);
}

/* Centered gate: always centered, honors notch/gesture insets, traps scroll. */
.center {
  align-items: center;
  padding: 1rem;
  padding-top: calc(1rem + var(--safe-area-inset-top, 0px));
  padding-bottom: calc(1rem + var(--safe-area-inset-bottom, 0px));
  overscroll-behavior: contain;
}

/* Bottom sheet: docked to the bottom on mobile, centered on ≥640px. */
.sheet {
  align-items: center;
  padding: 1rem;
}
@media (max-width: 639px) {
  .sheet {
    align-items: flex-end;
  }
}

.wrap {
  width: 100%;
}
.wrap.center {
  max-width: 420px;
}
.wrap.sheet {
  max-width: 30rem; /* max-w-120 */
}
</style>
