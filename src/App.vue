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

<style>
:root {
  font-family:
    -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue",
    Arial, sans-serif;
  font-size: 16px;
  line-height: 1.5;
  color: #1a1a2e;
  background-color: #f8f9fa;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
}

* {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

body {
  min-height: 100vh;
}

@media (prefers-color-scheme: dark) {
  :root {
    color: #e0e0e0;
    background-color: #1a1a2e;
  }
}
</style>
