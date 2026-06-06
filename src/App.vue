<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { onMounted, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";

const configured = ref(false);

onMounted(async () => {
  try {
    configured.value = await invoke<boolean>("is_configured");
  } catch {
    configured.value = false;
  }
});
</script>

<template>
  <router-view />
</template>
