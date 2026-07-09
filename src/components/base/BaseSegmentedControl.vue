<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts" generic="T">
// Segmented "pill" selector backed by sr-only radios (fieldset + legend keeps it
// accessible). `by` defaults to `===`; pass a custom comparator for object-valued
// options (e.g. the auto-lock presets `{ idle }`). Keys on `label`, so each
// group's labels must be unique.
const props = withDefaults(
  defineProps<{
    options: { label: string; value: T; labelClass?: string }[];
    legend?: string;
    name: string;
    modelValue: T;
    by?: (a: T, b: T) => boolean;
    disabled?: boolean;
    /** Allow pills to wrap onto multiple rows (long groups like auto-lock). */
    wrap?: boolean;
  }>(),
  { disabled: false, wrap: false },
);

const emit = defineEmits<{ (e: "change", value: T): void }>();

function isActive(v: T): boolean {
  return props.by ? props.by(props.modelValue, v) : props.modelValue === v;
}
function select(v: T) {
  emit("change", v);
}
</script>

<template>
  <fieldset class="border-0 p-0 m-0" :disabled="disabled">
    <legend v-if="legend" class="text-xs text-muted mb-1">{{ legend }}</legend>
    <div :class="['flex gap-2', { 'flex-wrap': wrap }]">
      <label
        v-for="opt in options"
        :key="opt.label"
        class="mode-pill"
        :class="{ 'mode-active': isActive(opt.value) }"
      >
        <input
          type="radio"
          :name="name"
          class="sr-only"
          :checked="isActive(opt.value)"
          @change="select(opt.value)"
        />
        <span :class="opt.labelClass">{{ opt.label }}</span>
      </label>
    </div>
    <slot name="hint" />
  </fieldset>
</template>

<style scoped>
.mode-pill {
  flex: 1;
  text-align: center;
  padding: 0.5rem 0.6rem;
  font-size: var(--text-sm);
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  background: var(--color-surface);
  cursor: pointer;
  -webkit-tap-highlight-color: transparent;
  min-height: 48px;
  display: flex;
  align-items: center;
  justify-content: center;
}
.mode-pill:active {
  background: var(--color-hover);
}
@media (hover: hover) {
  .mode-pill:hover {
    background: var(--color-hover);
  }
}
.mode-active {
  border-color: var(--color-accent);
  box-shadow: 0 0 0 2px var(--color-accent-ring);
}
</style>
