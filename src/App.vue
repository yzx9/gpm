<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { onMounted } from "vue";
import { applySafeAreaInsets } from "./utils/safe-area";
import { useLockState } from "./utils/useLockState";
import UnlockModal from "./components/UnlockModal.vue";

const { locked, ready, init } = useLockState();

onMounted(() => {
  applySafeAreaInsets();
  // init() reconciles `locked` with the backend's real state and flips `ready`.
  init();
});
</script>

<template>
  <div class="app-shell">
    <router-view />
    <!--
      Global unlock overlay: shown over whatever page is current when the
      identity is locked, so re-authentication happens in place. `locked` is a
      pure mirror of the backend (driven by its `identity-lock-state` events);
      `ready` just suppresses the overlay during the boot window before the
      first state is known. `locked` already implies "configured + encrypted".
    -->
    <UnlockModal v-if="ready && locked" />
  </div>
</template>
