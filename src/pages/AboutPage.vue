<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import AcknowledgementsSection from "@/components/about/AcknowledgementsSection.vue";
import LicensesSection from "@/components/about/LicensesSection.vue";
import OverviewSection from "@/components/about/OverviewSection.vue";
import BaseHeader from "@/components/base/BaseHeader.vue";
import BaseSegmentedControl from "@/components/base/BaseSegmentedControl.vue";
import { Info } from "@lucide/vue";
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

type Tab = "overview" | "acknowledgements" | "licenses";

const { t } = useI18n();

// Overview first — it's the page's real "front door" (what gpm is + design
// goals); acknowledgements and the full license tree follow.
const tab = ref<Tab>("overview");

// Computed so labels re-resolve when the active locale changes.
const TAB_OPTIONS = computed(
  () =>
    [
      { label: t("about.tabs.overview"), value: "overview" },
      { label: t("about.tabs.acknowledgements"), value: "acknowledgements" },
      { label: t("about.tabs.licenses"), value: "licenses" },
    ] as { label: string; value: Tab }[],
);

function onTabChange(v: Tab) {
  tab.value = v;
}
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <BaseHeader
      :back-fallback="{ name: 'settings' }"
      :title="t('about.title')"
      :title-icon="Info"
      spacing="sm"
    />

    <!-- Sticky tab bar so search/scrolling on the Licenses tab keeps the
         switcher reachable. -->
    <div class="tab-bar">
      <BaseSegmentedControl
        name="about-tabs"
        :model-value="tab"
        :options="TAB_OPTIONS"
        @change="onTabChange"
      />
    </div>

    <div class="mt-4">
      <OverviewSection v-if="tab === 'overview'" />
      <AcknowledgementsSection v-else-if="tab === 'acknowledgements'" />
      <LicensesSection v-else />
    </div>
  </main>
</template>

<style scoped>
.tab-bar {
  position: sticky;
  top: 0;
  z-index: 10;
  padding: 0.5rem 0;
  background: var(--color-bg, var(--color-surface));
}
</style>
