<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
// A plain-language, in-app summary of how gpm keeps secrets safe. The cards
// below are authored from the app's *current* behavior; docs/SECURITY.md is the
// detailed threat model for contributors (it can lag the app, so this page does
// not blindly mirror it). Reached from the Settings hub; carries no secret, so
// the route is not FLAG_SECURE (capturable, like About).
import BaseCard from "@/components/base/BaseCard.vue";
import BaseHeader from "@/components/base/BaseHeader.vue";
import { openExternal } from "@/utils/open-external";
import {
  BadgeCheck,
  ClipboardCopy,
  ExternalLink,
  Fingerprint,
  Lock,
  ShieldAlert,
  ShieldCheck,
  Smartphone,
  Timer,
} from "@lucide/vue";
import { useI18n } from "vue-i18n";

const { t } = useI18n();

// One icon per card; the title/body text resolves from the `security` locale
// namespace, which the router guard auto-loads for this route by name.
const CARDS = [
  { key: "local", icon: Smartphone },
  { key: "copyShow", icon: ClipboardCopy },
  { key: "autoLock", icon: Timer },
  { key: "atRest", icon: Lock },
  { key: "appLock", icon: Fingerprint },
  { key: "authenticity", icon: BadgeCheck },
  { key: "scope", icon: ShieldAlert },
] as const;

// Pinned to `main`: the doc is a living threat model, so the latest revision is
// the right target (it may temporarily outrun the installed build — acceptable,
// since the cards above are the at-version summary).
const SECURITY_DOC_URL =
  "https://github.com/yzx9/gpm/blob/main/docs/SECURITY.md";
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <BaseHeader
      :back-fallback="{ name: 'settings' }"
      :title="t('security.title')"
      :title-icon="ShieldCheck"
      spacing="sm"
    />

    <p class="intro">{{ t("security.intro") }}</p>

    <div class="cards">
      <BaseCard v-for="card in CARDS" :key="card.key" as="section">
        <h2 class="card-title">
          <component :is="card.icon" :size="16" />
          {{ t(`security.${card.key}.title`) }}
        </h2>
        <p class="card-body">{{ t(`security.${card.key}.body`) }}</p>
      </BaseCard>
    </div>

    <!-- Opens in the system browser via the opener plugin (tauri-plugin-opener);
         @click.prevent stops the WebView from navigating itself. `href` stays
         for semantics/accessibility and the dev-browser fallback. -->
    <a
      :href="SECURITY_DOC_URL"
      target="_blank"
      rel="noopener noreferrer"
      class="footer-link"
      data-testid="security-full-model-link"
      @click.prevent="openExternal(SECURITY_DOC_URL)"
    >
      <ExternalLink :size="14" /> {{ t("security.fullModelLink") }}
      <span class="sr-only">{{ t("common.opensInNewWindow") }}</span>
    </a>
  </main>
</template>

<style scoped>
.intro {
  font-size: var(--text-sm);
  color: var(--color-muted);
  margin-bottom: 1rem;
}
.cards {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}
.card-title {
  display: flex;
  align-items: center;
  gap: 0.25rem;
  font-size: var(--text-sm);
  font-weight: 500;
  margin-bottom: 0.5rem;
}
.card-body {
  font-size: var(--text-sm);
}
.footer-link {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 0.25rem;
  margin-top: 1rem;
  padding: 0.4rem 0.8rem;
  font-size: var(--text-sm);
  color: var(--color-accent);
  text-decoration: none;
  border-radius: var(--radius-md);
}
.footer-link:active {
  background: var(--color-hover);
}
@media (hover: hover) {
  .footer-link:hover {
    background: var(--color-hover);
  }
}
</style>
