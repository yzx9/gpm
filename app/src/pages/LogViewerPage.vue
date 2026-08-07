<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<script setup lang="ts">
import type { AppConfig, AppError } from "@/api";
import { clearLog, getAppConfig, readLog, setVerbose } from "@/api";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseCard from "@/components/base/BaseCard.vue";
import BaseHeader from "@/components/base/BaseHeader.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseSpinner from "@/components/base/BaseSpinner.vue";
import { useDiagnosticsExport, useDialog, useToast } from "@/composables";
import { Bug, Download, RefreshCw, ScrollText, Trash2 } from "@lucide/vue";
import { listen } from "@tauri-apps/api/event";
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

const { t } = useI18n();
const { toast } = useToast();
const { dialog } = useDialog();
const { exporting, runExport } = useDiagnosticsExport();

const logText = ref("");
const logPre = ref<HTMLPreElement | null>(null);
const loading = ref(false);
const clearing = ref(false);
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

/** State badge appended to the verbose toggle button — the countdown while the
 *  window is live, a marker once it has elapsed, nothing while off. */
const verboseBadge = computed(() => {
  if (verboseState.value === "on")
    return t("log.verboseRemaining", { remaining: remainingLabel.value });
  if (verboseState.value === "elapsed") return t("log.verboseElapsedBadge");
  return "";
});
/** Caption under the toggle. Only off (onboarding) and elapsed (needs
 *  explaining) surface one; while on, the badge carries the state. */
const verboseHint = computed(() => {
  if (verboseState.value === "off") return t("log.verboseOffHint");
  if (verboseState.value === "elapsed") return t("log.verboseElapsedHint");
  return "";
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

/** Newest entries accumulate at the bottom of the log, so pin the view there
 *  on load/refresh instead of defaulting to the top (oldest). */
function scrollToBottom() {
  const el = logPre.value;
  if (el) el.scrollTop = el.scrollHeight;
}
watch(logText, () => {
  void nextTick(scrollToBottom);
});

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
  const confirmed = await dialog.confirm({
    message: t("log.clearConfirm"),
    confirmLabel: t("common.button.clear"),
    danger: true,
  });
  if (!confirmed) return;
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
    />

    <BaseCard as="section" class="mb-4">
      <div class="flex flex-col gap-2">
        <!-- File actions. These lived in the header as three long labeled
             buttons that overflowed on narrow screens; moved to a full-width
             toolbar below the title. -->
        <div class="flex gap-2">
          <BaseButton
            class="flex-1"
            variant="ghost"
            :loading="loading"
            @click="load"
          >
            <BaseIcon :icon="RefreshCw" :size="16" />
            {{ t("log.refresh") }}
          </BaseButton>
          <BaseButton
            class="flex-1"
            variant="ghost"
            :loading="exporting"
            @click="() => runExport()"
          >
            <BaseIcon :icon="Download" :size="16" />
            {{ t("common.button.export") }}
          </BaseButton>
          <BaseButton
            class="flex-1"
            variant="ghost"
            :loading="clearing"
            :disabled="!logText"
            @click="onClear"
          >
            <BaseIcon :icon="Trash2" :size="16" />
            {{ t("common.button.clear") }}
          </BaseButton>
        </div>

        <!-- Verbose (Debug) toggle. Binary and already state-visible, so a single
             switch button rather than an On/Off option picker — tap flips it, the
             label carries the live countdown, and the caption only surfaces for
             off (onboarding) and elapsed (needs explaining). -->
        <BaseButton
          block
          :variant="verboseOn ? 'primary' : 'secondary'"
          :loading="verboseLoading"
          :aria-pressed="verboseOn"
          @click="onVerboseChange(!verboseOn)"
        >
          <BaseIcon :icon="Bug" :size="16" />
          {{ t("log.verboseLegend")
          }}<span v-if="verboseBadge"> · {{ verboseBadge }}</span>
        </BaseButton>
        <p v-if="verboseHint" class="text-xs text-muted">{{ verboseHint }}</p>
      </div>
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
    <pre v-else-if="logText" ref="logPre" class="log-display">{{
      logText
    }}</pre>
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
