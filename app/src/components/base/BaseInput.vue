<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { ref } from "vue";

// Single source of truth for the former `.input-base`. The model is
// `string | number` and the `.number` modifier (v-model.number) is honored via
// the set transformer, so numeric inputs (length/count) work the same as raw
// `<input v-model.number>`. Other attributes (id, autocomplete, autofocus,
// required, min/max, aria-*) fall through onto the input.
const [model, modifiers] = defineModel<string | number>({
  set(value) {
    if (!modifiers.number) return value;
    // Mirror Vue's built-in .number: parseFloat, and keep the raw input when it
    // isn't parseable (empty string, "abc") so clearing the field doesn't snap
    // to 0 — Number("")===0, but parseFloat("") is NaN.
    const n = parseFloat(value as string);
    return isNaN(n) ? value : n;
  },
});

withDefaults(
  defineProps<{
    type?: string;
    placeholder?: string;
    disabled?: boolean;
  }>(),
  { type: "text" },
);

// Expose focus() so parents can focus a v-if-revealed input. The native
// `autofocus` attribute only fires on initial document load, not when an input
// is mounted dynamically (e.g. a mode toggle revealing a passphrase field).
const inputRef = ref<HTMLInputElement | null>(null);
defineExpose({ focus: () => inputRef.value?.focus() });
</script>

<template>
  <input
    ref="inputRef"
    v-model="model"
    class="input"
    :type="type"
    :placeholder="placeholder"
    :disabled="disabled"
  />
</template>

<style scoped>
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
