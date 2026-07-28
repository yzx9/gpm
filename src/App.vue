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
import { onMounted, watch } from "vue";
import { useI18n } from "vue-i18n";
import AppLockOverlay from "./components/AppLockOverlay.vue";
import ToastHost from "./components/ToastHost.vue";
import UnlockModal from "./components/UnlockModal.vue";
import {
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
const { appLocked, appReady, init: initAppLock } = useAppLockState();
const {
  loadSecuritySettings,
  lockMode,
  reload: reloadSecurity,
} = useSecuritySettings();
// Activity bumper: any in-app tap/scroll/key extends the identity idle-lock
// timer (no-op under Immediate/Never; throttled; backend timer authoritative).
const lockActivity = createLockActivity(lockMode, identityCached);
const {
  initSecureScreen,
  setSecureOverlay,
  reload: reloadSecureScreen,
} = useSecureScreen();
// Drives the <router-view> slide transition: "slide-forward" on a push,
// "slide-back" on a pop, "" (instant) on secure↔non-secure boundaries and
// replace navigations. See useNavDirection for the secure-boundary gate.
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
      Stack-style slide between pages. No `mode="out-in"`: push/pop animate the
      departing and arriving pages simultaneously (iOS NavigationController
      feel). `:key="route.fullPath"` makes Vue treat each route as a distinct
      element so the transition fires on every nav. `transitionName` is "" on
      secure↔non-secure boundaries so FLAG_SECURE is never down while a secure
      page is still mid-leave (see useNavDirection + main.ts secure guard).
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
