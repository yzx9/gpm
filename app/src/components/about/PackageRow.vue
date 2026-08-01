<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<!-- One expandable dependency row. Shared by the grouped and flat (search)
     views so the row layout, aria, and license-text reveal live in one place.
     The license text is rendered only while expanded (v-if), so closed rows
     never pay for it — the tab's primary scale control. -->

<script setup lang="ts">
import type { LicensePackage } from "@/components/about/data";
import BaseIcon from "@/components/base/BaseIcon.vue";
import { ChevronDown, ChevronRight } from "@lucide/vue";
import { useI18n } from "vue-i18n";

defineProps<{ pkg: LicensePackage; expanded: boolean }>();
const emit = defineEmits<{ (e: "toggle"): void }>();
const { t } = useI18n();
</script>

<template>
  <li>
    <button
      type="button"
      class="pkg-row"
      :aria-expanded="expanded"
      :aria-label="t('about.licenses.expandAria', { name: pkg.name })"
      @click="emit('toggle')"
    >
      <BaseIcon :icon="expanded ? ChevronDown : ChevronRight" :size="14" />
      <span class="pkg-name">{{ pkg.name }}</span>
      <span class="pkg-version">{{ pkg.version }}</span>
      <span class="pkg-eco">{{ pkg.ecosystem }}</span>
    </button>
    <pre v-if="expanded" class="license-text">{{
      pkg.licenseText || t("about.licenses.noLicenseText")
    }}</pre>
  </li>
</template>

<style scoped>
.pkg-row {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 0.4rem;
  padding: 0.5rem 0.7rem;
  background: var(--color-input);
  border: 0;
  cursor: pointer;
  text-align: left;
  -webkit-tap-highlight-color: transparent;
}
.pkg-row:active {
  background: var(--color-hover);
}
@media (hover: hover) {
  .pkg-row:hover {
    background: var(--color-hover);
  }
}
.pkg-name {
  flex: 1;
  font-size: var(--text-sm);
  word-break: break-all;
}
.pkg-version {
  font-size: var(--text-xs);
  color: var(--color-muted, var(--color-edge));
  font-family: var(--font-mono, monospace);
}
.pkg-eco {
  font-size: 0.6rem;
  text-transform: uppercase;
  color: var(--color-muted, var(--color-edge));
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-sm);
  padding: 0 0.25rem;
  flex-shrink: 0;
}
.license-text {
  margin: 0;
  padding: 0.7rem;
  max-height: 16rem;
  overflow: auto;
  font-size: var(--text-xs);
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
  background: var(--color-surface);
  border-top: 1px solid var(--color-edge);
  font-family: var(--font-mono, monospace);
}
</style>
