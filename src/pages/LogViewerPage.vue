<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import type { AppError } from "@/api";
import { clearLog, getLogLevel, readLog, setLogLevel } from "@/api";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseHeader from "@/components/base/BaseHeader.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseSegmentedControl from "@/components/base/BaseSegmentedControl.vue";
import BaseSpinner from "@/components/base/BaseSpinner.vue";
import { useToast } from "@/composables";
import { RefreshCw, ScrollText, Trash2 } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

const { t } = useI18n();
const { toast } = useToast();

const logText = ref("");
const level = ref("info");
const loading = ref(false);
const clearing = ref(false);
const error = ref("");

const LEVELS = ["error", "warn", "info", "debug"] as const;
// Computed so the labels re-translate if the locale changes while the page is open.
const levelOptions = computed(() =>
  LEVELS.map((value) => ({ label: t(`log.levels.${value}`), value })),
);

onMounted(load);

/** Read the log + current level (refresh-on-open; no live tail — RFC 0052). */
async function load() {
  loading.value = true;
  error.value = "";
  // allSettled: a getLogLevel() failure must not hide the log text — readLog is
  // the load-bearing call (mirrors SettingsPage's loadSummaries resilience).
  const [textRes, lvlRes] = await Promise.allSettled([
    readLog(),
    getLogLevel(),
  ]);
  if (textRes.status === "fulfilled") {
    logText.value = textRes.value;
  } else {
    const appError = textRes.reason as AppError;
    error.value = appError?.message || t("log.loadFailed");
  }
  if (lvlRes.status === "fulfilled") {
    level.value = lvlRes.value;
  } // else: leave level as-is; the selector still works and next load re-reads
  loading.value = false;
}

/** Persist + apply the level at runtime (the backend calls `set_max_level`). */
async function onLevelChange(next: string) {
  level.value = next; // optimistic — the segmented control updates immediately
  try {
    await setLogLevel(next);
  } catch (e) {
    // Re-read the real backend level rather than reverting to a captured prev:
    // two rapid changes can interleave, and a stale prev would clobber a newer
    // optimistic value. The backend is the source of truth.
    try {
      level.value = await getLogLevel();
    } catch {
      // re-read also failed — leave the selector as-is (next load reconciles)
    }
    const appError = e as AppError;
    toast.danger(appError?.message || t("log.levelFailed"));
  }
}

async function onClear() {
  if (!confirm(t("log.clearConfirm"))) return;
  clearing.value = true;
  try {
    await clearLog();
    logText.value = "";
    toast.success(t("log.cleared"));
  } catch (e) {
    const appError = e as AppError;
    toast.danger(appError?.message || t("log.clearFailed"));
  } finally {
    clearing.value = false;
  }
}
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <BaseHeader
      :back-fallback="{ name: 'settings' }"
      :title="t('log.title')"
      :title-icon="ScrollText"
    >
      <template #actions>
        <BaseButton variant="ghost" :loading="loading" @click="load">
          <BaseIcon :icon="RefreshCw" :size="16" />
          {{ t("log.refresh") }}
        </BaseButton>
        <BaseButton
          variant="ghost"
          :loading="clearing"
          :disabled="!logText"
          @click="onClear"
        >
          <BaseIcon :icon="Trash2" :size="16" />
          {{ t("log.clear") }}
        </BaseButton>
      </template>
    </BaseHeader>

    <section class="mb-4">
      <BaseSegmentedControl
        :options="levelOptions"
        :model-value="level"
        name="log-level"
        :legend="t('log.levelLabel')"
        @change="onLevelChange"
      />
    </section>

    <BaseAlert v-if="error" variant="danger" class="mb-4">{{
      error
    }}</BaseAlert>

    <div
      v-if="loading && !logText"
      class="flex items-center gap-2 text-muted py-8"
    >
      <BaseSpinner />
    </div>
    <pre v-else-if="logText" class="log-display">{{ logText }}</pre>
    <p v-else class="text-muted text-sm">{{ t("log.empty") }}</p>
  </main>
</template>

<style scoped>
.log-display {
  padding: 0.6rem 0.75rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  background: var(--color-input);
  font-size: var(--text-xs);
  font-family: monospace;
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 60vh;
  overflow-y: auto;
  margin: 0;
}
</style>
