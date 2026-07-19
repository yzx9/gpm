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
/* No border + a recessed track bg: the pill reads as a sunken segment of the
   card rather than a second framed box nested inside it. The card already
   owns the edge border + radius, so echoing it on every pill produces a
   "box-in-a-box" frame; a tinted fill gives hierarchy without the redundancy. */
.mode-pill {
  flex: 1;
  text-align: center;
  padding: 0.5rem 0.6rem;
  font-size: var(--text-sm);
  border-radius: var(--radius-md);
  background: var(--color-input);
  cursor: pointer;
  -webkit-tap-highlight-color: transparent;
  /* .mode-pill is a <label>, so the global button user-select rule misses it;
     suppress long-press text selection on the pill labels here too. */
  -webkit-user-select: none;
  user-select: none;
  min-height: 48px;
  display: flex;
  align-items: center;
  justify-content: center;
  transition:
    background 0.15s,
    box-shadow 0.15s;
}
.mode-pill:not(.mode-active):active {
  background: var(--color-hover);
}
@media (hover: hover) {
  .mode-pill:not(.mode-active):hover {
    background: var(--color-hover);
  }
}
/* Selected segment "raises" out of the track (surface bg + accent edge + halo)
   entirely via box-shadow — no layout border, so toggling never shifts the
   pill by a border-width. */
.mode-active {
  background: var(--color-surface);
  box-shadow:
    inset 0 0 0 1px var(--color-accent),
    0 0 0 2px var(--color-accent-ring);
}
</style>
