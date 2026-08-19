<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<script setup lang="ts">
import type { AppConfig, AppError, UpdateStatus } from "@/api";
import { checkUpdateNow, setUpdateCheck } from "@/api";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseModalShell from "@/components/base/BaseModalShell.vue";
import { useToast } from "@/composables";
import { openExternal } from "@/utils/open-external";
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

// The About-page version dialog: the single surface for update-check config
// (RFC R090's On/Off card collapsed into the version affordance, so the About
// page itself stays purely informational). Every view is one primary + one
// explicit cancel (the app's confirm-dialog layout), with the auto-check
// toggle demoted to a quiet text link under the buttons that flips in place —
// the dialog never closes on a toggle, so the result of the action stays
// visible and a misclick is immediately reversible.
//
// Views:
//   checking  — a probe is running (buttons disabled; cancel stays live)
//   failed    — the manual probe errored: Retry (never claim "up to date"
//               without a completed probe — the backend fails loud here)
//   update    — a newer release is known (cached or just probed): Download
//   upToDate  — check on, nothing newer cached: Got it
//   off       — auto-check disabled: Check now (manual, ignores the pref)

const props = defineProps<{
  /** Current `update_check_enabled` (parent's app-config snapshot). */
  enabled: boolean;
  /** Cached probe status when the dialog opened (null before first load). */
  status: UpdateStatus | null;
}>();

const emit = defineEmits<{
  (e: "close"): void;
  /** A manual check completed (fresh result recorded); lets the parent
   *  refresh its dot even when the dialog was closed mid-flight. */
  (e: "checked"): void;
  (e: "pref-changed", config: AppConfig): void;
}>();

const { t } = useI18n();
const { toast } = useToast();

import { version } from "@/version";
const releasesUrl = "https://github.com/yzx9/gpm/releases/latest";

// Mirror of the pref so the footer link flips in place after a toggle instead
// of remounting the dialog. Synced back to the prop: the dialog can open
// before the parent's `getAppConfig` resolves (it passes the default-on
// mirror), and the real pref must win once it lands.
const prefEnabled = ref(props.enabled);
watch(
  () => props.enabled,
  (enabled) => {
    prefEnabled.value = enabled;
  },
);
// Fresh status once a manual check lands (null until then — the view falls
// back to the cached `status` prop).
const checked = ref<UpdateStatus | null>(null);
const checking = ref(false);
const failed = ref(false);
// Guards the footer link against rapid taps firing concurrent set_update_check
// calls (same get→mutate→save race the settings pages guard on).
const toggling = ref(false);
// Set on unmount: the dialog closes (backdrop/back are live during a check),
// but the in-flight toggle's continuation must not fire a post-close probe.
const closed = ref(false);
onBeforeUnmount(() => {
  closed.value = true;
});

// "Up to date" must never be claimed off missing data: with the check on and
// no successfully-probed tag (config still loading, or every probe so far
// failed — the backend reports both as latest_version null), probe now instead
// of rendering the up-to-date view.
onMounted(() => {
  if (prefEnabled.value && (props.status?.latest_version ?? null) === null) {
    void runCheck();
  }
});

type Mode = "checking" | "failed" | "update" | "upToDate" | "off";

const mode = computed<Mode>(() => {
  if (checking.value) return "checking";
  if (failed.value) return "failed";
  // A completed manual check always shows its result — even while the pref is
  // off (that's the off view's whole purpose), and it is NOT reset by the
  // cached-status rules below.
  if (checked.value) return checked.value.available ? "update" : "upToDate";
  if (!prefEnabled.value) return "off";
  return props.status?.available ? "update" : "upToDate";
});

const latestTag = computed(
  () => (checked.value ?? props.status)?.latest_version ?? "—",
);

const primaryLabel = computed(() => {
  if (checking.value) return t("about.updateDialog.checking");
  switch (mode.value) {
    case "update":
      return t("about.updateDialog.goToDownload");
    case "upToDate":
      return t("about.updateDialog.gotIt");
    case "failed":
      return t("common.button.retry");
    default:
      return t("about.updateDialog.checkNow");
  }
});

function onPrimary() {
  if (checking.value) return; // loading already disables; belt-and-braces
  switch (mode.value) {
    case "update":
      // Opens in the system browser via the opener plugin; the dialog closes
      // so returning to gpm lands back on About, not on a stale result view.
      void openExternal(releasesUrl);
      emit("close");
      break;
    case "upToDate":
      emit("close");
      break;
    default:
      void runCheck();
  }
}

async function runCheck() {
  if (checking.value || closed.value) return;
  checking.value = true;
  failed.value = false;
  try {
    checked.value = await checkUpdateNow();
    emit("checked");
  } catch {
    // Drop any stale success so a Retry starts from a clean view.
    checked.value = null;
    failed.value = true;
  } finally {
    checking.value = false;
  }
}

async function toggleAutoCheck() {
  if (toggling.value || checking.value) return;
  const next = !prefEnabled.value;
  toggling.value = true;
  try {
    const config = await setUpdateCheck(next);
    prefEnabled.value = next;
    emit("pref-changed", config);
    failed.value = false;
    // Enabling means "yes, check for updates" — but a result probed moments
    // ago is still fresh, and re-probing could regress it to a transient
    // failure; only probe when there is nothing to show.
    if (next && !checked.value) void runCheck();
  } catch (e) {
    const appError = e as AppError;
    toast.danger(appError?.message || t("about.updateDialog.toggleFailed"));
  } finally {
    toggling.value = false;
  }
}
</script>

<template>
  <BaseModalShell
    variant="center"
    :aria-label="t('about.updateDialog.title')"
    @close="emit('close')"
  >
    <h2 class="dialog-title" tabindex="-1">
      <template v-if="mode === 'checking'">{{
        t("about.updateDialog.title")
      }}</template>
      <template v-else-if="mode === 'failed'">{{
        t("about.updateDialog.failedTitle")
      }}</template>
      <template v-else-if="mode === 'update'">{{
        t("about.updateDialog.updateTitle")
      }}</template>
      <template v-else-if="mode === 'upToDate'">{{
        t("about.updateDialog.upToDateTitle")
      }}</template>
      <template v-else>{{ t("about.updateDialog.offTitle") }}</template>
    </h2>

    <p v-if="mode === 'update'" class="dialog-message">
      {{ t("about.updateDialog.updateBody") }}
    </p>
    <p v-else-if="mode === 'upToDate'" class="dialog-message">
      {{ t("about.updateDialog.upToDateBody", { version: `v${version}` }) }}
    </p>
    <p v-else-if="mode === 'off'" class="dialog-message">
      {{ t("about.updateDialog.offBody") }}
    </p>
    <p v-else-if="mode === 'failed'" class="dialog-message">
      {{ t("about.updateDialog.failedBody") }}
    </p>

    <!-- Update view: latest vs. running version, the numbers the dialog
         exists to show. Mono keeps tags scannable; latest carries the accent. -->
    <p v-if="mode === 'update'" class="version-lines">
      {{ t("about.updateDialog.latest") }}
      <span class="tag tag-latest">{{ latestTag }}</span>
      <span class="dot-sep">·</span>
      {{ t("about.updateDialog.current") }}
      <span class="tag">v{{ version }}</span>
    </p>

    <div class="dialog-actions">
      <BaseButton
        variant="primary"
        block
        :loading="checking"
        :disabled="toggling"
        @click="onPrimary"
      >
        {{ primaryLabel }}
      </BaseButton>
      <!-- Explicit cancel (the app's confirm-dialog convention — backdrop and
           Android back also close). Stays live while checking so a slow probe
           can be bailed on through a labeled control, not only the gestures. -->
      <BaseButton
        variant="outline"
        block
        :disabled="toggling"
        @click="emit('close')"
      >
        {{ t("common.button.cancel") }}
      </BaseButton>
    </div>

    <!-- The auto-check pref as a quiet toggle link — the one piece of config
         this dialog owns. Flips in place; never closes the dialog. -->
    <button
      type="button"
      class="pref-link"
      :disabled="checking || toggling"
      @click="toggleAutoCheck"
    >
      {{
        prefEnabled
          ? t("about.updateDialog.disable")
          : t("about.updateDialog.enable")
      }}
    </button>
  </BaseModalShell>
</template>

<style scoped>
/* Mirrors DialogHost's centered confirm: a tight message + two stacked
   full-width buttons (thumb-friendly on mobile, the primary on top). */
.dialog-title {
  font-size: var(--text-base);
  font-weight: 500;
  margin-bottom: 0.5rem;
}
.dialog-message {
  font-size: var(--text-sm);
  margin-bottom: 0.75rem;
}
.dialog-actions {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}
.version-lines {
  font-size: var(--text-sm);
  color: var(--color-muted);
  margin-bottom: 0.75rem;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.35rem;
}
.tag {
  font-family: var(--font-family-mono);
  font-size: var(--text-xs);
  padding: 0.15rem 0.4rem;
  border-radius: var(--radius-sm);
  background: var(--color-input);
  border: 1px solid var(--color-edge);
}
.tag-latest {
  color: var(--color-accent);
  border-color: var(--color-accent);
}
.dot-sep {
  color: var(--color-subtle);
}
/* Quiet in-dialog config link: reads as furniture, not an action button. */
.pref-link {
  display: block;
  margin: 0.75rem auto 0;
  background: none;
  border: none;
  padding: 0.25rem 0.5rem;
  font-size: var(--text-xs);
  color: var(--color-muted);
  text-decoration: underline;
  text-decoration-style: dotted;
  text-underline-offset: 3px;
  cursor: pointer;
}
.pref-link:active:not(:disabled) {
  color: var(--color-accent);
}
@media (hover: hover) {
  .pref-link:hover:not(:disabled) {
    color: var(--color-accent);
  }
}
.pref-link:disabled {
  opacity: 0.55;
  cursor: default;
}
</style>
