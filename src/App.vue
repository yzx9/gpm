<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { computed, onMounted, watch } from "vue";
import { applySafeAreaInsets } from "./utils/safe-area";
import {
  useAppLockState,
  useLockState,
  useOverlayBackHandler,
  useSecureScreen,
  useSecuritySettings,
  useToast,
} from "./composables";
import UnlockModal from "./components/UnlockModal.vue";
import AppLockOverlay from "./components/AppLockOverlay.vue";
import BaseToast from "./components/base/BaseToast.vue";

const { overlayUp, ready, init, cancelAuth } = useLockState();
const { appLocked, appReady, init: initAppLock } = useAppLockState();
const { loadSecuritySettings } = useSecuritySettings();
const { initSecureScreen, setSecureOverlay } = useSecureScreen();
const { toast } = useToast();

// The global unlock overlay collects the identity passphrase — a credential.
// Force FLAG_SECURE on whenever it's up, even on an otherwise-capturable route
// (e.g. /entries), and restore the route's level when it dismisses.
watch(overlayUp, (up) => {
  void setSecureOverlay(up);
});

// Capture the Android back button while the unlock overlay is up: back cancels a
// per-op auth prompt (cancelAuth dismisses it) or is consumed by a hard lock
// (cancelAuth is a no-op there, so the overlay stays and back can't escape).
// Mirrors the `v-if` source exactly so the handler is armed only while the
// overlay is actually rendered.
const overlayShown = computed(() => ready.value && overlayUp.value);
useOverlayBackHandler(overlayShown, cancelAuth);

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
    <router-view />
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
    <UnlockModal v-if="ready && overlayUp && !appLocked" />
    <!-- Global toast: app-shell messages (e.g. a screen-secure abort). -->
    <BaseToast v-if="toast" variant="danger">{{ toast }}</BaseToast>
  </div>
</template>
