<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
const model = defineModel<string>();
withDefaults(
  defineProps<{
    placeholder?: string;
    rows?: number | string;
    disabled?: boolean;
  }>(),
  { rows: 2 },
);
</script>

<template>
  <textarea
    v-model="model"
    class="input"
    :rows="rows"
    :placeholder="placeholder"
    :disabled="disabled"
  />
</template>

<style scoped>
/* Shares the `.input-base` look; callers add font-mono/text sizing via fallthrough
   class where needed (e.g. the trusted-key textarea). */
.input {
  padding: 0.6rem 0.75rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  font-size: var(--text-base);
  font-family: inherit;
  background: var(--color-input);
  color: inherit;
  min-height: 48px;
}
.input:focus {
  outline: none;
  border-color: var(--color-accent);
  box-shadow: 0 0 0 2px var(--color-accent-ring);
}
</style>
