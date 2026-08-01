<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { useDialog } from "@/composables";
import { useI18n } from "vue-i18n";
import BaseButton from "./base/BaseButton.vue";
import BaseModalShell from "./base/BaseModalShell.vue";

// Single app-wide dialog renderer. Mounts once in `App.vue` (beside
// `<ToastHost/>`) and turns the `useDialog()` queue into centered
// `BaseModalShell` confirms. This is the surface that retires the WebView's
// native `window.confirm()`: every confirm the user sees is now our UI.
//
// Each queued request renders as its own shell at the default overlay tier;
// a same-z second confirm stacks above a pending first by DOM order, and the
// back-handler registry dismisses the topmost (LIFO tie-break). Backdrop tap
// and Android back both route through the shell's `@close` → `resolve(false)`.
const { pending } = useDialog();
const { t } = useI18n();
</script>

<template>
  <BaseModalShell
    v-for="req in pending"
    :key="req.id"
    variant="center"
    role="alertdialog"
    :aria-label="req.opts.title || req.opts.message"
    @close="req.resolve(false)"
  >
    <h2
      v-if="req.opts.title"
      class="dialog-title"
      :class="{ 'text-danger': req.opts.danger }"
      tabindex="-1"
    >
      {{ req.opts.title }}
    </h2>
    <p class="dialog-message">{{ req.opts.message }}</p>
    <div class="dialog-actions">
      <BaseButton
        :variant="req.opts.danger ? 'danger' : 'primary'"
        block
        @click="req.resolve(true)"
      >
        {{ req.opts.confirmLabel ?? t("common.button.confirm") }}
      </BaseButton>
      <BaseButton variant="outline" block @click="req.resolve(false)">
        {{ req.opts.cancelLabel ?? t("common.button.cancel") }}
      </BaseButton>
    </div>
  </BaseModalShell>
</template>

<style scoped>
/* Mirrors DivergenceModal's centered confirm step: a tight message + two
   stacked full-width buttons (thumb-friendly on mobile, the primary action
   on top). */
.dialog-title {
  font-size: var(--text-base);
  font-weight: 500;
  margin-bottom: 0.5rem;
}
.dialog-message {
  font-size: var(--text-sm);
  margin-bottom: 0.75rem;
}
.dialog-actions {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}
</style>
