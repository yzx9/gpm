<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<script setup lang="ts">
import type { AppConfig, AppError, UpdateStatus } from "@/api";
import {
  acknowledgeUpdate,
  getAppConfig,
  getUpdateStatus,
  setUpdateCheck,
} from "@/api";
import { DESIGN_GOALS } from "@/components/about/data";
import BaseCard from "@/components/base/BaseCard.vue";
import BaseOnOffToggle from "@/components/base/BaseOnOffToggle.vue";
import { useToast } from "@/composables";
import { openExternal } from "@/utils/open-external";
import { ExternalLink, Heart, ShieldCheck, Target } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

const { t } = useI18n();
const { toast } = useToast();

// Version comes from the workspace package.json at build time (resolveJsonModule).
// The path is relative to this file's location under src/components/about/.
import pkg from "../../../package.json";
const version = pkg.version;
const repoUrl = "https://github.com/yzx9/gpm";
const releasesUrl = "https://github.com/yzx9/gpm/releases/latest";

// The core stack summarized on the Overview card. Kept short — the full,
// auto-scanned list lives on the Licenses tab.
const builtWith = ["Rust", "Tauri", "Vue 3", "age", "libgit2"] as const;

// RFC R090: passive update-availability check. The backend probes GitHub on cold
// start (≤1/day) and caches the result; this reads the cache (no network) to
// light a red dot beside the version + an Update link. Opening About acknowledges
// the current latest release so the Settings-entry dot falls quiet; this
// About-page dot ignores the ack and stays lit until the user updates.
const appConfig = ref<AppConfig | null>(null);
const updateStatus = ref<UpdateStatus | null>(null);
const hasUpdate = computed(() => updateStatus.value?.available ?? false);
const updateCheckEnabled = computed(
  () => appConfig.value?.update_check_enabled ?? true,
);
const updateCheckLoading = ref(false);

async function loadStatus() {
  try {
    const [cfg, status] = await Promise.all([
      getAppConfig(),
      getUpdateStatus(),
    ]);
    appConfig.value = cfg;
    updateStatus.value = status;
    // Opening About acknowledges the current latest release for this version —
    // the Settings-entry dot then falls quiet on its next mount.
    if (status.unacknowledged) void acknowledgeUpdate();
  } catch {
    // Fail-closed: a load error leaves the dot off. About still renders.
  }
}

async function onUpdateCheckChange(enabled: boolean) {
  if (!appConfig.value) return;
  updateCheckLoading.value = true;
  try {
    appConfig.value = await setUpdateCheck(enabled);
  } catch (e) {
    const appError = e as AppError;
    toast.danger(appError?.message || t("about.updateCheck.setFailed"));
  } finally {
    updateCheckLoading.value = false;
  }
}

onMounted(() => {
  void loadStatus();
});
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
      <div class="mt-2 flex items-center justify-center gap-1.5">
        <p class="text-xs text-muted">
          {{ t("about.overview.version") }} {{ version }}
        </p>
        <!-- RFC R090: persistent dot while a newer release exists (ignores the
             ack). Decorative — the Update link beside it is the labeled action. -->
        <span
          v-if="hasUpdate"
          class="update-dot"
          :title="t('about.overview.updateAvailable')"
          aria-hidden="true"
        />
      </div>
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
      <!-- RFC R090: shown only while a newer release is available. Opens the
           latest release page (APKs live there) in the system browser. -->
      <a
        v-if="hasUpdate"
        :href="releasesUrl"
        target="_blank"
        rel="noopener noreferrer"
        class="update-link mt-1 inline-flex items-center justify-center gap-1"
        @click.prevent="openExternal(releasesUrl)"
      >
        <ExternalLink :size="14" /> {{ t("about.overview.updateLink") }}
        <span class="sr-only">{{ t("common.opensInNewWindow") }}</span>
      </a>
    </BaseCard>

    <!-- RFC R090: toggle the passive update check on/off. -->
    <BaseCard as="section">
      <h2 class="text-sm font-medium mb-3">
        {{ t("about.updateCheck.title") }}
      </h2>
      <BaseOnOffToggle
        name="update-check"
        :legend="t('about.updateCheck.legend')"
        :model-value="updateCheckEnabled"
        :disabled="updateCheckLoading"
        @change="onUpdateCheckChange"
      >
        <template #hint>
          <p class="text-xs text-muted mt-1">
            {{ t("about.updateCheck.hint") }}
          </p>
        </template>
      </BaseOnOffToggle>
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
.update-link {
  font-size: var(--text-xs);
  color: var(--color-danger);
  font-weight: 500;
  text-decoration: none;
}
.update-link:active {
  opacity: 0.7;
}
@media (hover: hover) {
  .update-link:hover {
    opacity: 0.7;
  }
}
.update-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--color-danger);
  display: inline-block;
  flex-shrink: 0;
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
