<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import type { AppConfig, AppError, LockMode } from "@/api";
import {
  getAppConfig,
  setClipboardClearSecs,
  setLockMode,
  setViewClearSecs,
} from "@/api";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseCard from "@/components/base/BaseCard.vue";
import BaseHeader from "@/components/base/BaseHeader.vue";
import BaseSegmentedControl from "@/components/base/BaseSegmentedControl.vue";
import { useSecuritySettings } from "@/composables";
import { Lock } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

const { t } = useI18n();

const appConfig = ref<AppConfig | null>(null);
const loading = ref(false);
const error = ref("");
const lockLoading = ref(false);

const { applySecurityConfig } = useSecuritySettings();

// App auto-lock presets. "Immediate" (no-cache, per-op) is the default.
// Computed (not a plain const) so the labels can resolve through t().
const LOCK_PRESETS = computed<{ label: string; value: LockMode }[]>(() => [
  { label: t("settings.lock.immediate"), value: "immediate" },
  { label: t("settings.lock.minutes", { count: 1 }), value: { idle: 60 } },
  { label: t("settings.lock.minutes", { count: 5 }), value: { idle: 300 } },
  { label: t("settings.lock.minutes", { count: 15 }), value: { idle: 900 } },
  { label: t("settings.lock.minutes", { count: 30 }), value: { idle: 1800 } },
  { label: t("settings.lock.never"), value: "never" },
]);
// View-clear presets. A `null` value clears the override (tracks the default).
const VIEW_CLEAR_PRESETS = computed<{ label: string; value: number | null }[]>(
  () => [
    { label: t("settings.clear.seconds", { count: 10 }), value: 10 },
    { label: t("settings.clear.default", { count: 45 }), value: null },
    { label: t("settings.lock.minutes", { count: 3 }), value: 180 },
    { label: t("settings.lock.never"), value: 0 },
  ],
);
// Clipboard-clear presets. Same `null` ⇒ default convention.
const CLIPBOARD_CLEAR_PRESETS = computed<
  { label: string; value: number | null }[]
>(() => [
  { label: t("settings.clear.default", { count: 45 }), value: null },
  { label: t("settings.lock.minutes", { count: 3 }), value: 180 },
  { label: t("settings.lock.never"), value: 0 },
]);

const rawLockMode = computed<LockMode>(
  () => appConfig.value?.lock_mode ?? "immediate",
);
const rawViewClear = computed<number | null>(
  () => appConfig.value?.view_clear_secs ?? null,
);
const rawClipboardClear = computed<number | null>(
  () => appConfig.value?.clipboard_clear_secs ?? null,
);

// Two-arg equality for LockMode (handles the `{ idle }` object presets); passed
// to BaseSegmentedControl's `by` prop. `lockModeActive` (below) wraps it for the
// hint-line checks.
function lockModeEq(a: LockMode, b: LockMode): boolean {
  if (a === b) return true;
  if (typeof a === "object" && typeof b === "object") return a.idle === b.idle;
  return false;
}

function lockModeActive(p: LockMode): boolean {
  return lockModeEq(rawLockMode.value, p);
}

async function loadConfig() {
  loading.value = true;
  error.value = "";
  try {
    appConfig.value = await getAppConfig();
    applySecurityConfig(appConfig.value);
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("settings.lock.setModeFailed");
  } finally {
    loading.value = false;
  }
}

async function onLockModeChange(mode: LockMode) {
  if (!appConfig.value) return;
  lockLoading.value = true;
  error.value = "";
  try {
    appConfig.value = await setLockMode(mode);
    // Keep the reactive lockMode ref in sync so the activity bumper's filter
    // picks up the new mode immediately (mirrors onViewClearChange below).
    applySecurityConfig(appConfig.value);
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("settings.lock.setModeFailed");
  } finally {
    lockLoading.value = false;
  }
}

async function onViewClearChange(secs: number | null) {
  if (!appConfig.value) return;
  lockLoading.value = true;
  error.value = "";
  try {
    const updated = await setViewClearSecs(secs);
    appConfig.value = updated;
    applySecurityConfig(updated);
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("settings.clear.setViewFailed");
  } finally {
    lockLoading.value = false;
  }
}

async function onClipboardClearChange(secs: number | null) {
  if (!appConfig.value) return;
  lockLoading.value = true;
  error.value = "";
  try {
    appConfig.value = await setClipboardClearSecs(secs);
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("settings.clear.setClipboardFailed");
  } finally {
    lockLoading.value = false;
  }
}

onMounted(() => {
  loadConfig();
});
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <BaseHeader
      :back-fallback="{ name: 'settings' }"
      :title="t('settings.hub.locking')"
      :title-icon="Lock"
    />

    <div v-if="loading" class="text-center text-muted py-8">
      {{ t("common.loading") }}
    </div>

    <BaseAlert v-else-if="error" variant="danger" class="mb-4">
      {{ error }}
    </BaseAlert>

    <div v-else-if="appConfig" class="flex flex-col gap-4">
      <!-- Auto-lock & auto-clear -->
      <BaseCard as="section">
        <h2 class="text-sm font-medium mb-3">
          {{ t("settings.lock.title") }}
        </h2>
        <p class="text-xs text-muted mb-3">
          {{ t("settings.lock.description") }}
        </p>

        <!-- App auto-lock mode -->
        <BaseSegmentedControl
          class="mb-3"
          name="lock-mode"
          :legend="t('settings.lock.autoLockLegend')"
          wrap
          :model-value="rawLockMode"
          :by="lockModeEq"
          :options="LOCK_PRESETS"
          :disabled="lockLoading"
          @change="onLockModeChange"
        >
          <template #hint>
            <p class="text-xs text-muted mt-1">
              <template v-if="lockModeActive('immediate')">{{
                t("settings.lock.immediateHint")
              }}</template>
              <template v-else-if="lockModeActive('never')">{{
                t("settings.lock.neverHint")
              }}</template>
              <template v-else>{{ t("settings.lock.idleHint") }}</template>
            </p>
          </template>
        </BaseSegmentedControl>

        <!-- View auto-clear -->
        <BaseSegmentedControl
          class="mb-3"
          name="view-clear"
          :legend="t('settings.lock.viewClearLegend')"
          wrap
          :model-value="rawViewClear"
          :options="VIEW_CLEAR_PRESETS"
          :disabled="lockLoading"
          @change="onViewClearChange"
        />

        <!-- Clipboard auto-clear -->
        <BaseSegmentedControl
          name="clipboard-clear"
          :legend="t('settings.lock.clipboardClearLegend')"
          wrap
          :model-value="rawClipboardClear"
          :options="CLIPBOARD_CLEAR_PRESETS"
          :disabled="lockLoading"
          @change="onClipboardClearChange"
        />
      </BaseCard>
    </div>
  </main>
</template>
