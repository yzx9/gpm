<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { ACKNOWLEDGEMENTS } from "@/components/about/data";
import BaseCard from "@/components/base/BaseCard.vue";
import { openExternal } from "@/utils/open-external";
import { ExternalLink } from "@lucide/vue";
import { useI18n } from "vue-i18n";

const { t } = useI18n();
</script>

<template>
  <div class="flex flex-col gap-4">
    <p class="text-sm text-muted px-1">{{ t("about.ack.intro") }}</p>

    <BaseCard
      v-for="ack in ACKNOWLEDGEMENTS"
      :key="ack.id"
      as="section"
      variant="raised"
    >
      <div class="flex items-baseline justify-between gap-2 mb-1">
        <!-- Name opens the project in the system browser via the opener plugin;
             @click.prevent stops the WebView from navigating itself. -->
        <a
          :href="ack.url"
          target="_blank"
          rel="noopener noreferrer"
          class="ack-name inline-flex items-center gap-1"
          @click.prevent="openExternal(ack.url)"
        >
          {{ ack.name }}
          <ExternalLink :size="13" class="text-muted" />
          <span class="sr-only">{{ t("common.opensInNewWindow") }}</span>
        </a>
        <span class="ack-license">{{ ack.license }}</span>
      </div>
      <p class="text-sm">{{ t(ack.descKey) }}</p>
    </BaseCard>
  </div>
</template>

<style scoped>
.ack-name {
  font-size: var(--text-base);
  font-weight: 600;
  color: var(--color-accent);
  text-decoration: none;
}
.ack-license {
  font-size: var(--text-xs);
  color: var(--color-muted, var(--color-edge));
  padding: 0.1rem 0.4rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-sm);
  white-space: nowrap;
  flex-shrink: 0;
}
</style>
