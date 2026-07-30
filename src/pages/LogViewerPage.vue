<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import type { AppConfig, AppError } from "@/api";
import {
  clearLog,
  exportDiagnostics,
  getAppConfig,
  readLog,
  setVerbose,
} from "@/api";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseCard from "@/components/base/BaseCard.vue";
import BaseHeader from "@/components/base/BaseHeader.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseSegmentedControl from "@/components/base/BaseSegmentedControl.vue";
import BaseSpinner from "@/components/base/BaseSpinner.vue";
import { useToast } from "@/composables";
import { Download, RefreshCw, ScrollText, Trash2 } from "@lucide/vue";
import { listen } from "@tauri-apps/api/event";
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useI18n } from "vue-i18n";

const { t } = useI18n();
const { toast } = useToast();

const logText = ref("");
const loading = ref(false);
const clearing = ref(false);
const exporting = ref(false);
const error = ref("");

// Verbose (Debug) toggle state. `verbose_until` is a Unix-seconds deadline set
// by `set_verbose`; the level is Debug while it is live. A backend deadline
// timer auto-reverts to Info when the window elapses — mid-session, emitting
// `verbose-reverted`, which this page listens for (onVerboseReverted) to flip
// the toggle Off. If the process is killed first, the next launch clears the
// expired deadline at startup.
const appConfig = ref<AppConfig | null>(null);
const verboseLoading = ref(false);
// Ticked once a second while verbose is active, so the countdown + active/expired
// hint stay live. Vanilla setInterval (the app has no VueUse dep); cleared on
// unmount and when verbose goes inactive.
const nowTick = ref(Date.now());
let countdownTimer: ReturnType<typeof setInterval> | null = null;

/** Toggle position: On while a deadline is set (Debug this session). */
const verboseOn = computed(() => appConfig.value?.verbose_until != null);
/** Hint phase: `on` while the window is live, `elapsed` once it has passed. */
const verboseState = computed<"off" | "on" | "elapsed">(() => {
  const v = appConfig.value?.verbose_until;
  if (v == null) return "off";
  return v * 1000 > nowTick.value ? "on" : "elapsed";
});
/** Remaining window as `m:ss`, recomputed each tick. */
const remainingLabel = computed(() => {
  const v = appConfig.value?.verbose_until;
  if (typeof v !== "number") return "";
  const secs = Math.max(0, v - Math.floor(nowTick.value / 1000));
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return `${m}:${s.toString().padStart(2, "0")}`;
});

function startCountdown() {
  if (countdownTimer) return;
  countdownTimer = setInterval(() => {
    nowTick.value = Date.now();
    // Stop ticking once the window has elapsed (the hint flips to "elapsed").
    if (verboseState.value !== "on") stopCountdown();
  }, 1000);
}
function stopCountdown() {
  if (countdownTimer) {
    clearInterval(countdownTimer);
    countdownTimer = null;
  }
}

/** Backend auto-reverted verbose to Info (the deadline elapsed). Re-read the
 *  config rather than clearing locally: a toggle-On that landed between the
 *  revert and this queued event would otherwise be clobbered, leaving the UI
 *  showing Off while the backend logs at Debug. The App shell posts the OS
 *  notification off the same event. */
async function onVerboseReverted() {
  stopCountdown();
  try {
    appConfig.value = await getAppConfig();
  } catch {
    // best-effort — the next load/visibility re-syncs the toggle
  }
}

let disposed = false;
let verboseRevertedUnlisten: (() => void) | null = null;

onMounted(() => {
  void load();
  void listen<null>("verbose-reverted", onVerboseReverted)
    .then((un) => {
      // If the page unmounted before listen resolved, unlisten immediately.
      if (disposed) un();
      else verboseRevertedUnlisten = un;
    })
    .catch(() => {});
});
onUnmounted(() => {
  disposed = true;
  verboseRevertedUnlisten?.();
  verboseRevertedUnlisten = null;
  stopCountdown();
});

/** Read the log + the verbose state (refresh-on-open; no live tail). */
async function load() {
  loading.value = true;
  error.value = "";
  try {
    logText.value = await readLog();
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("log.loadFailed");
  } finally {
    loading.value = false;
  }
  // Verbose state is secondary — a fetch failure must not block the log view.
  try {
    appConfig.value = await getAppConfig();
    if (verboseState.value === "on") startCountdown();
  } catch {
    // leave the toggle at its default (off); non-fatal
  }
}

async function onVerboseChange(enabled: boolean) {
  if (verboseLoading.value || enabled === verboseOn.value) return;
  verboseLoading.value = true;
  try {
    appConfig.value = await setVerbose(
      enabled,
      // Stage the localized revert-notification text (the backend posts it from
      // Rust when the window elapses, so it fires even when backgrounded).
      enabled
        ? {
            title: t("log.verboseNotifTitle"),
            body: t("log.verboseRevertedNotifBody"),
          }
        : undefined,
    );
    // Refresh the tick before reading `remainingLabel`: the interval only ticks
    // while a countdown is already running, so at enable-time `nowTick` can be
    // stale from mount and would inflate the toast's remaining time.
    nowTick.value = Date.now();
    if (verboseState.value === "on") {
      // Notify immediately: verbose is now on for the bounded window.
      toast.info(
        t("log.verboseActiveToast", { remaining: remainingLabel.value }),
      );
      startCountdown();
    } else {
      stopCountdown();
    }
  } catch (e) {
    const appError = e as AppError;
    toast.danger(appError?.message || t("log.loadFailed"));
  } finally {
    verboseLoading.value = false;
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

async function onExport() {
  if (!confirm(t("log.exportConfirm"))) return;
  exporting.value = true;
  try {
    await exportDiagnostics();
    toast.success(t("log.exported"));
  } catch (e) {
    const appError = e as AppError;
    // A dismissed save dialog is a silent cancel, not an error.
    if (appError?.code === "CANCELLED") return;
    toast.danger(appError?.message || t("log.exportFailed"));
  } finally {
    exporting.value = false;
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
        <BaseButton variant="ghost" :loading="exporting" @click="onExport">
          <BaseIcon :icon="Download" :size="16" />
          {{ t("log.export") }}
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

    <BaseCard as="section" class="mb-4">
      <BaseSegmentedControl
        name="verbose"
        :legend="t('log.verboseLegend')"
        :model-value="verboseOn"
        :options="[
          { label: t('log.verboseOn'), value: true },
          { label: t('log.verboseOff'), value: false },
        ]"
        :disabled="verboseLoading"
        @change="onVerboseChange"
      >
        <template #hint>
          <p class="text-xs text-muted mt-1">
            <template v-if="verboseState === 'on'">{{
              t("log.verboseOnHint", { remaining: remainingLabel })
            }}</template>
            <template v-else-if="verboseState === 'elapsed'">{{
              t("log.verboseElapsedHint")
            }}</template>
            <template v-else>{{ t("log.verboseOffHint") }}</template>
          </p>
        </template>
      </BaseSegmentedControl>
    </BaseCard>

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
