<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
/** The gpm logo mark. Renders the placeholder raster (public/icon-*.png) until
 * the seal logo from docs/design.md §4 lands; swapping this one component swaps
 * every brand-mark surface (root header, setup, about). Decorative — the "gpm"
 * wordmark beside it carries the name, so the image is aria-hidden. */
withDefaults(defineProps<{ size?: number }>(), { size: 24 });
</script>

<template>
  <img
    src="/icon-128.png"
    alt=""
    aria-hidden="true"
    class="brand-mark"
    :width="size"
    :height="size"
    :style="{ width: `${size}px`, height: `${size}px` }"
  />
</template>

<style scoped>
.brand-mark {
  display: block;
  flex: none;
  border-radius: var(--radius-sm);
  object-fit: contain;
}
</style>
