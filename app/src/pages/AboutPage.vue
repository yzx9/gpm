<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<script setup lang="ts">
import type { AppConfig, UpdateStatus } from "@/api";
import { acknowledgeUpdate, getAppConfig, getUpdateStatus } from "@/api";
import { DESIGN_GOALS } from "@/components/about/data";
import UpdateDialog from "@/components/about/UpdateDialog.vue";
import BaseCard from "@/components/base/BaseCard.vue";
import BaseHeader from "@/components/base/BaseHeader.vue";
import { openExternal } from "@/utils/open-external";
import { version } from "@/version";
import {
  ChevronRight,
  ExternalLink,
  Heart,
  ShieldCheck,
  Target,
} from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

const { t } = useI18n();

const repoUrl = "https://github.com/yzx9/gpm";

// The core stack summarized on the Overview card. Kept short — the full,
// auto-scanned list lives on the Licenses screen.
const builtWith = ["Rust", "Tauri", "Vue 3", "age", "libgit2"] as const;

// RFC R090: passive update-availability check. The backend probes GitHub on cold
// start (≤1/day) and caches the result; this reads the cache (no network) to
// light a red dot beside the version. Opening About acknowledges the current
// latest release so the Settings-entry dot falls quiet; this About-page dot
// ignores the ack and stays lit until the user updates. All update-check config
// and the download link live in the version dialog, keeping this page purely
// informational.
const appConfig = ref<AppConfig | null>(null);
const updateStatus = ref<UpdateStatus | null>(null);
const hasUpdate = computed(() => updateStatus.value?.available ?? false);
const updateCheckEnabled = computed(
  () => appConfig.value?.update_check_enabled ?? true,
);
const updateDialogOpen = ref(false);

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

// The dialog may have changed the pref or probed fresh (manual check), so the
// dot re-reads the cache on close instead of trusting the mount-time snapshot.
async function refreshStatus() {
  try {
    updateStatus.value = await getUpdateStatus();
  } catch {
    // Keep the mount-time snapshot — the dot is advisory.
  }
}

async function onUpdateDialogClose() {
  updateDialogOpen.value = false;
  await refreshStatus();
}

onMounted(() => {
  void loadStatus();
});
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <BaseHeader
      :back-fallback="{ name: 'settings' }"
      :title="t('about.title')"
      spacing="sm"
    />

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
        <!-- RFC R090: the version is the update-check entry — tapping it opens the
             version dialog (download link, manual check, auto-check pref). The
             persistent dot beside it stays lit while a newer release exists
             (ignores the ack); it's decorative, the button is the labeled action. -->
        <!-- No aria-label override: the visible "Version vX.Y.Z" text is the
             accessible name (WCAG 2.5.3 Label in Name). -->
        <button
          type="button"
          class="version-btn mt-2"
          @click="updateDialogOpen = true"
        >
          <span class="text-xs text-muted">
            {{ t("about.overview.version") }} {{ version }}
          </span>
          <span
            v-if="hasUpdate"
            class="update-dot"
            :title="t('about.overview.updateAvailable')"
            aria-hidden="true"
          />
          <ChevronRight :size="12" class="version-chevron" aria-hidden="true" />
        </button>
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

    <!-- RFC R090: update status + the auto-check pref, collapsed into the
         version entry above. -->
    <UpdateDialog
      v-if="updateDialogOpen"
      :enabled="updateCheckEnabled"
      :status="updateStatus"
      @close="onUpdateDialogClose"
      @checked="refreshStatus"
      @pref-changed="appConfig = $event"
    />
  </main>
</template>

<style scoped>
.app-icon {
  width: 72px;
  height: 72px;
  border-radius: var(--radius-md);
}
/* Version entry into the update dialog — styled as quiet inline text with a
   chevron affordance, not a loud button, so the hero stays informational.
   Tap-highlight/user-select come from the global `button` rule in style.css. */
.version-btn {
  display: inline-flex;
  align-items: center;
  gap: 0.375rem;
  background: none;
  border: none;
  padding: 0.3rem 0.6rem;
  margin-left: -0.6rem; /* optically center the text despite the chevron */
  min-height: 36px; /* chip-button floor (cf. BaseButton size-xs) */
  border-radius: var(--radius-md);
  cursor: pointer;
}
.version-btn:active {
  background: var(--color-hover);
}
@media (hover: hover) {
  .version-btn:hover {
    background: var(--color-hover);
  }
}
.version-chevron {
  color: var(--color-subtle);
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
