<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { applySafeAreaInsets } from "./utils/safe-area";

const configured = ref(false);

onMounted(() => {
  applySafeAreaInsets();
  invoke<boolean>("is_configured")
    .then((v) => {
      configured.value = v;
    })
    .catch(() => {
      configured.value = false;
    });
});
</script>

<template>
  <div class="app-shell">
    <router-view />
  </div>
</template>
