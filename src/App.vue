<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { computed, onMounted } from "vue";
import { applySafeAreaInsets } from "./utils/safe-area";
import { useLockState } from "./utils/useLockState";
import { useOverlayBackHandler } from "./utils/useOverlayBackHandler";
import { useSecuritySettings } from "./utils/useSecuritySettings";
import UnlockModal from "./components/UnlockModal.vue";

const { overlayUp, ready, init, cancelAuth } = useLockState();
const { loadSecuritySettings } = useSecuritySettings();

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
  // Prime the view-clear cache so the first reveal uses the configured timer.
  loadSecuritySettings();
});
</script>

<template>
  <div class="app-shell">
    <router-view />
    <!--
      Global unlock overlay: shown over whatever page is current when the
      identity needs authentication — either a hard lock (manual/idle) or a
      per-operation auth prompt (Immediate no-cache mode). `overlayUp` covers
      both; `ready` suppresses the overlay during the boot window before the
      first state is known. An overlay implies "configured + encrypted".
    -->
    <UnlockModal v-if="ready && overlayUp" />
  </div>
</template>
