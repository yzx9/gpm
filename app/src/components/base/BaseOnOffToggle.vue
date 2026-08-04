<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
// Binary On/Off pill — a BaseSegmentedControl with the shared On/Off options
// baked in, so the boolean settings across the app share one source of truth for
// the labels, their order (On left, Off right), and the `common.toggle.*` locale
// key. `class` / `style` / extra attrs pass through to the underlying fieldset,
// and the `#hint` slot forwards verbatim. The options are a computed (not a
// const) so the labels re-resolve when the display language changes live.
import BaseSegmentedControl from "@/components/base/BaseSegmentedControl.vue";
import { computed } from "vue";
import { useI18n } from "vue-i18n";

withDefaults(
  defineProps<{
    name: string;
    modelValue: boolean;
    legend?: string;
    ariaLabel?: string;
    disabled?: boolean;
  }>(),
  { disabled: false },
);

const emit = defineEmits<{ (e: "change", value: boolean): void }>();

const { t } = useI18n();

const options = computed(() => [
  { label: t("common.toggle.on"), value: true },
  { label: t("common.toggle.off"), value: false },
]);

function onChange(value: boolean) {
  emit("change", value);
}
</script>

<template>
  <BaseSegmentedControl
    :name="name"
    :legend="legend"
    :aria-label="ariaLabel"
    :model-value="modelValue"
    :options="options"
    :disabled="disabled"
    @change="onChange"
  >
    <template #hint><slot name="hint" /></template>
  </BaseSegmentedControl>
</template>
