<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import {
  getAppConfig,
  isVerboseActive,
  notifyOs,
  verboseRemainingSecs,
} from "@/api";
import { computed, onMounted, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import AppLockOverlay from "./components/AppLockOverlay.vue";
import ToastHost from "./components/ToastHost.vue";
import UnlockModal from "./components/UnlockModal.vue";
import {
  createForegroundSyncStore,
  createLockActivity,
  useAppLockState,
  useLockState,
  useNavDirection,
  useSecureScreen,
  useSecuritySettings,
} from "./composables";
import { applySafeAreaInsets } from "./utils/safe-area";

const {
  overlayUp,
  ready,
  init,
  dismissOverlay,
  identityCached,
  shouldAutoPromptBiometric,
} = useLockState();
const {
  appLockEnabled,
  appLocked,
  appReady,
  init: initAppLock,
} = useAppLockState();
const {
  gateIdle,
  loadSecuritySettings,
  lockMode,
  reload: reloadSecurity,
} = useSecuritySettings();
// True when the gate idle timer is armed (gate on + unlocked + gate-idle != Off)
// so the activity bumper resets it on in-app use, not just on secret ops.
const gateIdleArmed = computed(
  () => appLockEnabled.value && !appLocked.value && gateIdle.value !== "off",
);
// Activity bumper: any in-app tap/scroll/key extends BOTH idle timers (identity
// + gate); no-op when neither is armed (Immediate/Never + gate-idle Off/locked).
const lockActivity = createLockActivity(
  lockMode,
  identityCached,
  gateIdleArmed,
);
// Best-effort foreground sync (RFC R060 Tier 1): pull + push on cold-start/resume
// when AutoSync is on; surfaces divergence / Enforce-block as a passive status
// badge, never a modal; silent on success and failure.
const foregroundSync = createForegroundSyncStore(
  useAppLockState(),
  useRouter(),
);
const {
  initSecureScreen,
  setSecureOverlay,
  reload: reloadSecureScreen,
} = useSecureScreen();
// Drives the <router-view> slide transition: "slide-forward" on a push,
// "slide-back" on a pop, "" (instant) on the initial paint and on replace
// navigations. Screen-capture protection is component-level (R031), so there
// is no secure↔capturable boundary to freeze on — every navigation animates.
const { transitionName } = useNavDirection();
const { t } = useI18n();

// Both credential overlays — the identity UnlockModal (`overlayUp`) and the
// app-launch AppLockOverlay (`appLocked`) — must force FLAG_SECURE on whenever
// either is up, even on an otherwise-capturable route (e.g. /entries) and even
// under screen-mode "off". `setSecureOverlay` drives `desiredSecure`'s
// `overlayActive`, which every mode secures. Combined so the two overlays can't
// clobber each other's secure state.
watch([overlayUp, appLocked], ([up, locked]) => {
  void setSecureOverlay(up || locked);
});

// On a real app-unlock (locked→false) the sealed behavior config is now readable
// on the backend; reload the security + secure-screen caches (their cold-start
// load ran under the app-lock overlay and read defaults). The backend loads
// behavior + reseeds autosync before emitting locked:false, so the real values
// are ready. `appLocked` is a clean boolean from backend events (the resume
// re-lock debounce doesn't flap it), so this fires once per real unlock.
watch(appLocked, (locked, prev) => {
  if (prev && !locked) {
    void reloadSecurity();
    void reloadSecureScreen();
  }
});

/** Format the verbose window's remaining seconds as `m:ss`. */
function formatRemaining(secs: number): string {
  return `${Math.floor(secs / 60)}:${(secs % 60).toString().padStart(2, "0")}`;
}

/**
 * If verbose logging is still active at boot (the user relaunched inside the
 * window), surface an OS notification so they know — Debug is on and reverts to
 * Info at the deadline. Sent via the system notification channel (not an in-app
 * toast) so it isn't hidden under the unlock/app-lock overlay. Best-effort.
 */
async function notifyVerboseOnBoot() {
  try {
    const deadline = (await getAppConfig()).verbose_until;
    if (!isVerboseActive(deadline)) return;
    void notifyOs(
      t("log.verboseNotifTitle"),
      t("log.verboseBootNotifBody", {
        remaining: formatRemaining(verboseRemainingSecs(deadline)),
      }),
    );
  } catch {
    // non-fatal — best-effort boot notice
  }
}

onMounted(() => {
  applySafeAreaInsets();
  // init() reconciles `locked` with the backend's real state and flips `ready`.
  init();
  // init the app-launch gate state (no-op when the gate is off / on desktop).
  initAppLock();
  // Arm the foreground-sync resume listener + cold-start sync (R060 Tier 1).
  foregroundSync.init();
  // Prime the view-clear cache so the first reveal uses the configured timer.
  loadSecuritySettings();
  // Start extending the identity idle-lock timer on in-app activity (Idle mode).
  lockActivity.init();
  // Load the screen-capture master toggle + platform availability, then
  // reconcile FLAG_SECURE for the current route. The boot default in
  // MainActivity.onCreate keeps every screen secure until this runs.
  initSecureScreen();
  // Surface a notice if a verbose session is still active from a prior launch.
  void notifyVerboseOnBoot();
});
</script>

<template>
  <div class="app-shell">
    <!-- Unified toast host: top-of-shell, in-flow. Renders the useToast queue
         once for every caller (pages + app-shell code like the router guard). -->
    <ToastHost />
    <!--
      Foreground-sync attention badge (R060 Tier 1): a passive, persistent
      indicator that a foreground sync hit a divergence / Enforce block. Tap takes
      the user to the entry list, where a pull-to-refresh engages the existing
      resolve flow — the sync itself never opens a modal or enters
      conflict-resolution. The colored word carries the meaning (no icon pill).
    -->
    <button
      v-if="foregroundSync.syncAttention.value"
      class="sync-attention"
      type="button"
      :aria-label="t('common.sync.attentionHint')"
      :title="t('common.sync.attentionHint')"
      @click="foregroundSync.engage()"
    >
      {{ t("common.sync.attentionBadge") }}
    </button>
    <!--
      Stack-style slide between pages. No `mode="out-in"`: push/pop animate the
      departing and arriving pages simultaneously (iOS NavigationController
      feel). `:key="route.fullPath"` makes Vue treat each route as a distinct
      element so the transition fires on every nav. `transitionName` is "" only
      on the initial paint and on replace navigations — screen-capture
      protection is component-level (R031), so there is no secure↔capturable
      boundary to freeze the slide on (see useNavDirection).
    -->
    <router-view v-slot="{ Component, route }">
      <Transition :name="transitionName">
        <component :is="Component" :key="route.fullPath" />
      </Transition>
    </router-view>
    <!--
      App-launch biometric gate overlay: shown over everything while the
      seal master key is not in memory (cold start with the gate on, or after
      a background re-lock). Sits above the identity UnlockModal (z-index 70 vs
      60) and suppresses it while up, so the two gates never race to show
      competing prompts.
    -->
    <AppLockOverlay v-if="appReady && appLocked" />
    <!--
      Identity unlock overlay: shown over whatever page is current when the
      identity needs authentication — either a hard lock (manual/idle) or a
      per-operation auth prompt (Immediate no-cache mode). `overlayUp` covers
      both; `ready` suppresses it during the boot window; `!appLocked` suppresses
      it while the app-launch gate overlay is up.
    -->
    <UnlockModal
      v-if="ready && overlayUp && !appLocked"
      :auto-prompt-biometric="shouldAutoPromptBiometric"
      @close="dismissOverlay"
    />
  </div>
</template>

<style scoped>
/* Foreground-sync attention badge — a small colored word (RFC R060 Tier 1).
   Reuses the warning tokens (which dark-adapt via both prefers-color-scheme and
   [data-theme="dark"]), matching the rest of the codebase. Placement is a
   best-effort top-center pill; a design pass can refine it. */
.sync-attention {
  position: fixed;
  top: calc(env(safe-area-inset-top) + 0.5rem);
  left: 50%;
  transform: translateX(-50%);
  z-index: 40;
  min-height: 44px;
  padding: 0 1rem;
  display: inline-flex;
  align-items: center;
  border: none;
  border-radius: 999px;
  background: var(--color-warning-soft);
  color: var(--color-warning);
  font-size: 0.8rem;
  font-weight: 600;
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.25);
}
</style>
