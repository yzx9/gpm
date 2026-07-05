<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { computed, onMounted, watch } from "vue";
import AppLockOverlay from "./components/AppLockOverlay.vue";
import ToastHost from "./components/ToastHost.vue";
import UnlockModal from "./components/UnlockModal.vue";
import {
  useAppLockState,
  useLockState,
  useNavDirection,
  useOverlayBackHandler,
  useSecureScreen,
  useSecuritySettings,
} from "./composables";
import { applySafeAreaInsets } from "./utils/safe-area";

const { overlayUp, ready, init, dismissOverlay } = useLockState();
const { appLocked, appReady, init: initAppLock } = useAppLockState();
const { loadSecuritySettings } = useSecuritySettings();
const { initSecureScreen, setSecureOverlay } = useSecureScreen();
// Drives the <router-view> slide transition: "slide-forward" on a push,
// "slide-back" on a pop, "" (instant) on secure↔non-secure boundaries and
// replace navigations. See useNavDirection for the secure-boundary gate.
const { transitionName } = useNavDirection();

// The global unlock overlay collects the identity passphrase — a credential.
// Force FLAG_SECURE on whenever it's up, even on an otherwise-capturable route
// (e.g. /entries), and restore the route's level when it dismisses.
watch(overlayUp, (up) => {
  void setSecureOverlay(up);
});

// Capture the Android back button while the unlock overlay is up: back dismisses
// it — a per-op auth is cancelled (rejecting parked callers), and a hard lock is
// hidden WITHOUT unlocking (the identity stays locked; the next secret op
// re-prompts). Mirrors the `v-if` source exactly so the handler is armed only
// while the overlay is actually rendered.
const overlayShown = computed(() => ready.value && overlayUp.value);
useOverlayBackHandler(overlayShown, dismissOverlay);

onMounted(() => {
  applySafeAreaInsets();
  // init() reconciles `locked` with the backend's real state and flips `ready`.
  init();
  // init the app-launch gate state (no-op when the gate is off / on desktop).
  initAppLock();
  // Prime the view-clear cache so the first reveal uses the configured timer.
  loadSecuritySettings();
  // Load the screen-capture master toggle + platform availability, then
  // reconcile FLAG_SECURE for the current route. The boot default in
  // MainActivity.onCreate keeps every screen secure until this runs.
  initSecureScreen();
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
      at-rest master key is not in memory (cold start with the gate on, or after
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
      @close="dismissOverlay"
    />
  </div>
</template>
