<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<script setup lang="ts">
import BaseCard from "@/components/base/BaseCard.vue";
import BaseHeader from "@/components/base/BaseHeader.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import {
  ChevronRight,
  Database,
  Info,
  Lock,
  ScrollText,
  ShieldCheck,
  SlidersHorizontal,
} from "@lucide/vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";

import pkg from "../../package.json";

const router = useRouter();
const { t } = useI18n();

// The hub is a pure navigation menu — each category's own page holds the
// detail, so a one-line summary here only added clutter. About's summary is
// the installed version (a constant, no load).
const version = pkg.version;
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <BaseHeader
      :back-fallback="{ name: 'entries' }"
      :title="t('settings.title')"
    />

    <BaseCard as="section" class="hub">
      <div
        class="hub-row"
        tabindex="0"
        role="button"
        :aria-label="t('settings.hub.general')"
        @click="router.push({ name: 'settingsGeneral' })"
        @keydown.enter="router.push({ name: 'settingsGeneral' })"
        @keydown.space.prevent="router.push({ name: 'settingsGeneral' })"
      >
        <BaseIcon :icon="SlidersHorizontal" :size="20" class="text-muted" />
        <span class="hub-title">{{ t("settings.hub.general") }}</span>
        <BaseIcon :icon="ChevronRight" :size="20" class="text-muted" />
      </div>

      <div
        class="hub-row"
        tabindex="0"
        role="button"
        :aria-label="t('settings.hub.lockAndIdentity')"
        @click="router.push({ name: 'settingsIdentity' })"
        @keydown.enter="router.push({ name: 'settingsIdentity' })"
        @keydown.space.prevent="router.push({ name: 'settingsIdentity' })"
      >
        <BaseIcon :icon="Lock" :size="20" class="text-muted" />
        <span class="hub-title">{{ t("settings.hub.lockAndIdentity") }}</span>
        <BaseIcon :icon="ChevronRight" :size="20" class="text-muted" />
      </div>

      <div
        class="hub-row"
        tabindex="0"
        role="button"
        :aria-label="t('settings.hub.repository')"
        @click="router.push({ name: 'settingsRepository' })"
        @keydown.enter="router.push({ name: 'settingsRepository' })"
        @keydown.space.prevent="router.push({ name: 'settingsRepository' })"
      >
        <BaseIcon :icon="Database" :size="20" class="text-muted" />
        <span class="hub-title">{{ t("settings.hub.repository") }}</span>
        <BaseIcon :icon="ChevronRight" :size="20" class="text-muted" />
      </div>

      <!-- Diagnostics log viewer — leads the docs group (Logs/Security/
           Permissions/About) that follows the settings categories. The app
           logs at the fixed Info default, so there is no per-row level
           summary. Independent of repo/identity state, so always shown. -->
      <div
        class="hub-row"
        tabindex="0"
        role="button"
        :aria-label="t('settings.hub.logs')"
        @click="router.push({ name: 'log' })"
        @keydown.enter="router.push({ name: 'log' })"
        @keydown.space.prevent="router.push({ name: 'log' })"
      >
        <BaseIcon :icon="ScrollText" :size="20" class="text-muted" />
        <span class="hub-title">{{ t("settings.hub.logs") }}</span>
        <BaseIcon :icon="ChevronRight" :size="20" class="text-muted" />
      </div>

      <!-- Security: plain-language explainer of how gpm protects secrets.
           Carries no secret content, so (like About) it sits below the four
           category pages. -->
      <div
        class="hub-row"
        tabindex="0"
        role="button"
        :aria-label="t('settings.hub.security')"
        @click="router.push({ name: 'security' })"
        @keydown.enter="router.push({ name: 'security' })"
        @keydown.space.prevent="router.push({ name: 'security' })"
      >
        <BaseIcon :icon="ShieldCheck" :size="20" class="text-muted" />
        <span class="hub-title">{{ t("settings.hub.security") }}</span>
        <BaseIcon :icon="ChevronRight" :size="20" class="text-muted" />
      </div>

      <!-- Permissions & data: what gpm accesses, why, and a deep-link to the
           system toggle for permissions Android suppresses after two denials.
           Carries no secret, so (like Security) sits below the category pages. -->
      <div
        class="hub-row"
        tabindex="0"
        role="button"
        :aria-label="t('settings.hub.permissions')"
        @click="router.push({ name: 'settingsPermissions' })"
        @keydown.enter="router.push({ name: 'settingsPermissions' })"
        @keydown.space.prevent="router.push({ name: 'settingsPermissions' })"
      >
        <BaseIcon :icon="ShieldCheck" :size="20" class="text-muted" />
        <span class="hub-title">{{ t("settings.hub.permissions") }}</span>
        <BaseIcon :icon="ChevronRight" :size="20" class="text-muted" />
      </div>

      <!-- About: overview, acknowledgements, and the open-source license
           tree. Carries no secret content (not a settings category), so it
           sits below the four category pages. Its summary value is the
           installed version. -->
      <div
        class="hub-row"
        tabindex="0"
        role="button"
        :aria-label="`${t('settings.hub.about')} — ${version}`"
        @click="router.push({ name: 'about' })"
        @keydown.enter="router.push({ name: 'about' })"
        @keydown.space.prevent="router.push({ name: 'about' })"
      >
        <BaseIcon :icon="Info" :size="20" class="text-muted" />
        <span class="hub-title">{{ t("settings.hub.about") }}</span>
        <span class="hub-value">{{ version }}</span>
        <BaseIcon :icon="ChevronRight" :size="20" class="text-muted" />
      </div>
    </BaseCard>
  </main>
</template>

<style scoped>
.hub {
  padding: 0.25rem 1rem;
}
.hub-row {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  min-height: 3rem; /* 48px touch target */
  padding: 0.6rem 0;
  cursor: pointer;
  border-radius: var(--radius-sm);
  transition: background-color 0.15s;
}
.hub-row + .hub-row {
  border-top: 1px solid var(--color-edge);
}
.hub-row:focus-visible {
  background: var(--color-hover, var(--color-edge));
  outline: none;
}
@media (hover: hover) {
  .hub-row:hover {
    background: var(--color-hover, var(--color-edge));
  }
}
.hub-title {
  font-size: 0.95rem;
  /* Push the summary value + chevron to the right edge whether the row carries
     a value or not (e.g. Logs has none), so every chevron stays aligned. */
  margin-right: auto;
}
.hub-value {
  font-size: 0.8rem;
  color: var(--color-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 50%;
}
</style>
