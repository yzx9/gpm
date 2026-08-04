<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts" generic="T">
// Single-select that opens a bottom sheet (mobile) / centered card (≥640px) of
// options. Mirrors BaseSegmentedControl's API AND its selection pattern: a
// <fieldset> of sr-only radios, so the browser gives us keyboard nav (arrow/Tab)
// and aria-checked for free — no hand-rolled focus engine. Arrow keys and taps
// both commit on change, so picking an option selects + closes the sheet (there
// is no browse-without-commit). The sheet
// itself is BaseModalShell (variant="sheet"), which already supplies backdrop
// tap, scroll lock, Android back, and z-index tiering.
//
// Focus contract (BaseModalShell does NOT manage focus for its caller): on open,
// focus moves to the checked radio so screen readers announce the entry point and
// ESC/arrows work; on close (every path flips `open` false) focus returns to the
// trigger so the user never loses their place. ESC + a minimal Tab wrap keep
// focus inside the sheet while it's open.
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseModalShell from "@/components/base/BaseModalShell.vue";
import { Check, ChevronDown } from "@lucide/vue";
import { computed, nextTick, ref, watch } from "vue";

const props = withDefaults(
  defineProps<{
    options: { label: string; value: T }[];
    modelValue: T;
    /** Radio-group name; also derives the legend/dialog ids. */
    name: string;
    /** Inline label above the trigger; also the dialog's accessible name. */
    legend?: string;
    /** Accessible name when `legend` is absent (one of the two is required). */
    ariaLabel?: string;
    /** Trigger text (muted) shown when nothing is selected / matches. */
    placeholder?: string;
    /** Text shown inside the sheet when `options` is empty. */
    emptyLabel?: string;
    /** Equality for object-valued options; defaults to `===`. */
    by?: (a: T, b: T) => boolean;
    disabled?: boolean;
  }>(),
  { disabled: false },
);

const emit = defineEmits<{ (e: "change", value: T): void }>();

const open = ref(false);
const trigger = ref<HTMLButtonElement | null>(null);
const sheet = ref<HTMLFieldSetElement | null>(null);

function isActive(v: T): boolean {
  return props.by ? props.by(props.modelValue, v) : props.modelValue === v;
}

const selectedLabel = computed(
  () => props.options.find((o) => isActive(o.value))?.label ?? "",
);

// Accessible name for the trigger (when there's no legend to label it) and the
// dialog. One of `legend` / `ariaLabel` should be provided.
const accessibleName = computed(() => props.legend ?? props.ariaLabel ?? "");

function pick(v: T) {
  emit("change", v);
  open.value = false; // optimistic close; the parent's persistence is async
}

// Focus the checked radio (or the first) when the sheet opens; restore focus to
// the trigger when it closes. Both arms nextTick so the DOM (un)mounts first.
watch(open, (v, prev) => {
  if (v) {
    void nextTick(() => {
      const root = sheet.value;
      if (!root) return;
      const radios = root.querySelectorAll<HTMLInputElement>(
        'input[type="radio"]',
      );
      const target = Array.from(radios).find((r) => r.checked) ?? radios[0];
      // Fall back to the fieldset itself when there are no options, so ESC and
      // the Tab wrap still have a focused target inside the sheet.
      (target ?? root).focus();
    });
  } else if (prev) {
    void nextTick(() => trigger.value?.focus());
  }
});

// ESC closes; Tab/Shift+Tab cycle among the options so focus can't leave the
// sheet for the trigger behind the overlay. Events bubble from the focused
// radio up to the fieldset.
function onSheetKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") {
    e.preventDefault();
    open.value = false;
    return;
  }
  if (e.key !== "Tab") return;
  // Always trap Tab inside the sheet: cycle among the radios (or hold focus on
  // the fieldset when there are none).
  e.preventDefault();
  const radios = sheet.value
    ? Array.from(
        sheet.value.querySelectorAll<HTMLInputElement>('input[type="radio"]'),
      )
    : [];
  if (radios.length === 0) {
    sheet.value?.focus();
    return;
  }
  const idx = radios.findIndex((r) => r === document.activeElement);
  const dir = e.shiftKey ? -1 : 1;
  radios[(idx + dir + radios.length) % radios.length]!.focus();
}
</script>

<template>
  <div class="base-select">
    <span v-if="legend" :id="`${name}-legend`" class="legend">{{
      legend
    }}</span>
    <button
      ref="trigger"
      type="button"
      class="trigger"
      :disabled="disabled"
      :inert="open"
      :aria-expanded="open"
      :aria-controls="open ? `${name}-sheet` : undefined"
      :aria-labelledby="legend ? `${name}-legend` : undefined"
      :aria-label="legend ? undefined : accessibleName"
      @click="open = true"
    >
      <span class="trigger-label" :class="{ placeholder: !selectedLabel }">
        {{ selectedLabel || placeholder }}
      </span>
      <BaseIcon :icon="ChevronDown" :size="16" class="chevron" />
    </button>
    <slot name="hint" />

    <BaseModalShell
      v-if="open"
      variant="sheet"
      :aria-label="accessibleName"
      @close="open = false"
    >
      <fieldset
        :id="`${name}-sheet`"
        ref="sheet"
        class="options"
        :disabled="disabled"
        tabindex="-1"
        @keydown="onSheetKeydown"
      >
        <label
          v-for="opt in options"
          :key="opt.label"
          class="option"
          :class="{ active: isActive(opt.value) }"
        >
          <input
            type="radio"
            class="sr-only"
            :name="name"
            :checked="isActive(opt.value)"
            @change="pick(opt.value)"
          />
          <span class="option-label">{{ opt.label }}</span>
          <BaseIcon
            v-if="isActive(opt.value)"
            :icon="Check"
            :size="18"
            class="check"
          />
        </label>
        <p v-if="options.length === 0 && emptyLabel" class="no-options">
          {{ emptyLabel }}
        </p>
      </fieldset>
    </BaseModalShell>
  </div>
</template>

<style scoped>
.legend {
  display: block;
  font-size: var(--text-xs);
  color: var(--color-muted);
  margin-bottom: 0.25rem;
}

/* Trigger mimics BaseInput's `.input` field so the select reads as a sibling of
   the other inputs, not a foreign control. */
.trigger {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem;
  width: 100%;
  padding: 0.6rem 0.75rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  font-size: var(--text-base);
  font-family: inherit;
  background: var(--color-input);
  color: inherit;
  min-height: 48px;
  cursor: pointer;
  -webkit-tap-highlight-color: transparent;
}
.trigger:focus-visible {
  outline: none;
  border-color: var(--color-accent);
  box-shadow: 0 0 0 2px var(--color-accent-ring);
}
/* Every pressable owns a themed :active (tap-highlight is globally transparent),
   mirroring BaseSegmentedControl.mode-pill. */
.trigger:not(:disabled):active {
  background: var(--color-hover);
}
@media (hover: hover) {
  .trigger:not(:disabled):hover {
    background: var(--color-hover);
  }
}
.trigger:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

.trigger-label {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  text-align: start;
}
.trigger-label.placeholder {
  color: var(--color-muted);
}
.chevron {
  flex: none;
  color: var(--color-muted);
}

.options {
  border: 0;
  padding: 0;
  margin: 0;
  display: flex;
  flex-direction: column;
  /* Cap the list so a long option set (e.g. password templates) scrolls inside
     the sheet instead of overflowing the viewport. */
  max-height: calc(100dvh - 8rem);
  overflow-y: auto;
}
.option {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  min-height: 48px;
  padding: 0.5rem 0.25rem;
  font-size: var(--text-base);
  cursor: pointer;
  -webkit-tap-highlight-color: transparent;
  border-bottom: 1px solid var(--color-edge);
}
.option:last-of-type {
  border-bottom: 0;
}
.option-label {
  flex: 1 1 auto;
  min-width: 0;
  /* Wrap long/unknown-length labels to two lines rather than truncating, so a
     template name stays readable when this component is reused. */
  display: -webkit-box;
  -webkit-line-clamp: 2;
  line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
/* Selected row: accent text + a translucent accent fill (the --color-accent-ring
   tint already used for the input focus halo), stronger than text-only. */
.option.active {
  color: var(--color-accent);
  background: var(--color-accent-ring);
}
.check {
  flex: none;
  color: currentColor;
}
.no-options {
  padding: 0.75rem 0.25rem;
  font-size: var(--text-base);
  color: var(--color-muted);
}
</style>
