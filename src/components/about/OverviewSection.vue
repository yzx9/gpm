<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { DESIGN_GOALS } from "@/components/about/data";
import BaseCard from "@/components/base/BaseCard.vue";
import { openExternal } from "@/utils/open-external";
import { ExternalLink, Heart, ShieldCheck, Target } from "@lucide/vue";
import { useI18n } from "vue-i18n";

const { t } = useI18n();

// Version comes from the workspace package.json at build time (resolveJsonModule).
// The path is relative to this file's location under src/components/about/.
import pkg from "../../../package.json";
const version = pkg.version;
const repoUrl = "https://github.com/yzx9/gpm";

// The core stack summarized on the Overview card. Kept short — the full,
// auto-scanned list lives on the Licenses tab.
const builtWith = ["Rust", "Tauri", "Vue 3", "age", "libgit2"] as const;
</script>

<template>
  <div class="flex flex-col gap-4">
    <!-- Identity -->
    <BaseCard as="section" variant="raised" class="text-center">
      <img
        src="/icon-512.png"
        alt=""
        aria-hidden="true"
        class="app-icon mx-auto mb-3"
        width="72"
        height="72"
      />
      <h2 class="text-lg font-semibold">gpm</h2>
      <p class="text-sm text-muted mt-1">{{ t("about.overview.tagline") }}</p>
      <p class="text-xs text-muted mt-2">
        {{ t("about.overview.version") }} {{ version }}
      </p>
      <!-- Opens in the system browser via the opener plugin (tauri-plugin-opener);
           @click.prevent stops the WebView from navigating itself. `href` stays
           for semantics/accessibility and the dev-browser fallback. -->
      <a
        :href="repoUrl"
        target="_blank"
        rel="noopener noreferrer"
        class="repo-link mt-3 inline-flex items-center justify-center gap-1"
        @click.prevent="openExternal(repoUrl)"
      >
        <ExternalLink :size="14" /> {{ t("about.overview.repoLink") }}
        <span class="sr-only">{{ t("common.opensInNewWindow") }}</span>
      </a>
    </BaseCard>

    <!-- Design goals -->
    <BaseCard as="section">
      <h2 class="text-sm font-medium mb-3 flex items-center gap-1">
        <Target :size="16" /> {{ t("about.overview.designGoalsTitle") }}
      </h2>
      <ul class="flex flex-col gap-2">
        <li
          v-for="goal in DESIGN_GOALS"
          :key="goal"
          class="flex items-start gap-2 text-sm"
        >
          <ShieldCheck :size="16" class="goal-check shrink-0" />
          <span>{{ t(`about.overview.goals.${goal}`) }}</span>
        </li>
      </ul>
    </BaseCard>

    <!-- Built with -->
    <BaseCard as="section">
      <h2 class="text-sm font-medium mb-3 flex items-center gap-1">
        <Heart :size="16" /> {{ t("about.overview.builtWithTitle") }}
      </h2>
      <div class="flex flex-wrap gap-2">
        <span v-for="tech in builtWith" :key="tech" class="tech-chip">{{
          tech
        }}</span>
      </div>
    </BaseCard>
  </div>
</template>

<style scoped>
.app-icon {
  width: 72px;
  height: 72px;
  border-radius: var(--radius-md);
}
.repo-link {
  font-size: var(--text-sm);
  color: var(--color-accent);
  text-decoration: none;
  padding: 0.4rem 0.8rem;
  border-radius: var(--radius-md);
}
.repo-link:active {
  background: var(--color-hover);
}
@media (hover: hover) {
  .repo-link:hover {
    background: var(--color-hover);
  }
}
.goal-check {
  color: var(--color-success, var(--color-accent));
  margin-top: 0.1rem;
}
.tech-chip {
  font-size: var(--text-xs);
  padding: 0.25rem 0.6rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-sm);
  background: var(--color-surface);
}
</style>
