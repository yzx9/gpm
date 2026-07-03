<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import type { LucideIcon } from "@lucide/vue";
import { computed, useAttrs } from "vue";

/** Thin standardizing wrapper around a Lucide icon (mirrors the official
 * `IconButton` pattern in @lucide/vue's TS guide: declare only `icon`, forward
 * the rest). `icon` is typed with the library's own `LucideIcon`; every other
 * Lucide prop (`size`, `strokeWidth`, `color`, `class`, `aria-*`, …) falls
 * through to the underlying component, so this stays in sync with the library
 * instead of redeclaring its surface. (`& LucideProps` in the props type would
 * make `@vue/compiler-sfc` choke resolving `Partial<SVGAttributes>` at build.)
 *
 * Centralized defaults (`size: 20`, `strokeWidth: 1.75`) match the 36/48px
 * touch targets and the minimal outline aesthetic of the `Base*` family; the
 * `$attrs` spread binds after the defaults, so a caller's `:size="40"` wins.
 * `@lucide/vue` 1.x does not set `aria-hidden` itself, so this wrapper marks
 * the icon decorative unless the caller gives it an accessible name
 * (`aria-label` / `aria-labelledby` / `title`). */
defineOptions({ inheritAttrs: false });
const props = defineProps<{ icon: LucideIcon }>();
const attrs = useAttrs();

const decorative = computed(
  () => !(attrs["aria-label"] || attrs["aria-labelledby"] || attrs["title"]),
);
</script>

<template>
  <component
    :is="props.icon"
    :size="20"
    :stroke-width="1.75"
    :aria-hidden="decorative ? 'true' : undefined"
    v-bind="attrs"
  />
</template>
