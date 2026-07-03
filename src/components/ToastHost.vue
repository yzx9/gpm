<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { useToast } from "@/composables";
import { X } from "@lucide/vue";
import BaseAlert from "./base/BaseAlert.vue";
import BaseIcon from "./base/BaseIcon.vue";

// Single app-wide toast renderer. Mounts once in `App.vue` (above
// `<router-view/>`) so even app-shell callers like the router guard have a
// host. Reuses `BaseAlert` so toasts share the exact visuals of other inline
// banners; this component owns only the queue layout, enter/leave motion, and
// the optional × button (shown when an item is `closable`).
const { toasts, toast } = useToast();
</script>

<template>
  <div class="toast-host" :class="{ 'toast-host--empty': !toasts.length }">
    <TransitionGroup name="toast">
      <BaseAlert
        v-for="t in toasts"
        :key="t.id"
        :variant="t.variant"
        class="toast-host__item flex items-center gap-2"
      >
        <span class="flex-1">{{ t.message }}</span>
        <button
          v-if="t.closable"
          type="button"
          class="toast-host__close"
          aria-label="Close"
          @click="toast.dismiss(t.id)"
        >
          <BaseIcon :icon="X" :size="14" />
        </button>
      </BaseAlert>
    </TransitionGroup>
  </div>
</template>

<style scoped>
.toast-host {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  /* Sit at the very top of the app shell (above the page header) with a little
     breathing room; safe-area top inset is already applied by `.app-shell`. */
  padding: 0.5rem 1rem 0;
}

/* Empty queue: the host stays mounted (so the last toast's leave transition
   completes instead of being ripped out by a `v-if`) but reserves no top gap. */
.toast-host--empty {
  padding: 0;
}

.toast-host__close {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 24px;
  min-height: 24px;
  padding: 0;
  background: transparent;
  border: none;
  color: inherit;
  cursor: pointer;
}

.toast-enter-active,
.toast-leave-active {
  transition:
    opacity 0.18s ease,
    transform 0.18s ease;
}
.toast-enter-from,
.toast-leave-to {
  opacity: 0;
  transform: translateY(-0.25rem);
}
</style>
