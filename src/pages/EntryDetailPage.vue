<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { ref, onBeforeUnmount, onMounted } from "vue";
import { useRoute, useRouter } from "vue-router";
import { invoke } from "@tauri-apps/api/core";
import type { SensitiveContent, AppError } from "../types";

const route = useRoute();
const router = useRouter();

const entryPath = decodeURIComponent(
  Array.isArray(route.params.pathMatch)
    ? route.params.pathMatch[0]
    : route.params.pathMatch,
);
const entryName = entryPath.replace(/\.age$/, "");

// Sensitive state — must be nulled on page leave and auto-cleared
const password = ref<string | null>(null);
const notes = ref<string | null>(null);
const loading = ref(false);
const error = ref("");
const revealed = ref(false);
let autoHideTimer: ReturnType<typeof setTimeout> | null = null;

async function showPassword() {
  loading.value = true;
  error.value = "";
  try {
    const result = await invoke<SensitiveContent>("show_password", {
      entryPath,
    });
    password.value = result.password;
    notes.value = result.notes;
    revealed.value = true;

    // Auto-clear after 30 seconds
    clearSensitive();
    autoHideTimer = setTimeout(() => {
      clearSensitive();
    }, 30_000);
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Decryption failed";
  } finally {
    loading.value = false;
  }
}

async function copyPassword() {
  error.value = "";
  try {
    const result = await invoke<import("../types").CopyResult>(
      "copy_password",
      {
        entryPath,
      },
    );
    error.value = ""; // clear any previous error
    // Show brief feedback
    revealed.value = false;
    notes.value = null;
    password.value = `Copied! (${result.cleared_after_secs}s auto-clear)`;
    setTimeout(() => {
      password.value = null;
    }, 3000);
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Copy failed";
  }
}

function clearSensitive() {
  password.value = null;
  notes.value = null;
  revealed.value = false;
  if (autoHideTimer) {
    clearTimeout(autoHideTimer);
    autoHideTimer = null;
  }
}

function goBack() {
  clearSensitive();
  router.push({ name: "entries" });
}

// Clean up on page leave — security lifecycle
onBeforeUnmount(() => {
  clearSensitive();
});

// Also handle browser back navigation
onMounted(() => {
  window.addEventListener("popstate", clearSensitive);
});

onBeforeUnmount(() => {
  window.removeEventListener("popstate", clearSensitive);
});
</script>

<template>
  <div class="entry-detail-page">
    <header class="header">
      <button @click="goBack" class="btn-back">← Back</button>
      <h1 class="entry-title">{{ entryName }}</h1>
    </header>

    <div v-if="error" class="error">{{ error }}</div>

    <div class="actions">
      <button @click="copyPassword" class="btn-primary" :disabled="loading">
        📋 Copy Password
      </button>
      <button @click="showPassword" class="btn-secondary" :disabled="loading">
        {{ revealed ? "👁 Showing..." : "👁 Show Password" }}
      </button>
    </div>

    <div v-if="loading" class="loading">Decrypting...</div>

    <div v-if="revealed && password !== null" class="sensitive-section">
      <div class="field">
        <label>Password</label>
        <div class="password-display">{{ password }}</div>
      </div>

      <div v-if="notes" class="field">
        <label>Notes</label>
        <pre class="notes-display">{{ notes }}</pre>
      </div>

      <p class="auto-clear-hint">Auto-clears in 30 seconds</p>
    </div>
  </div>
</template>

<style scoped>
.entry-detail-page {
  max-width: 480px;
  margin: 0 auto;
  padding: 1rem;
}

.header {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  margin-bottom: 1.5rem;
}

.btn-back {
  background: none;
  border: none;
  font-size: 1rem;
  cursor: pointer;
  color: #4a6cf7;
  padding: 0.25rem;
}

.entry-title {
  font-size: 1.1rem;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1;
}

.error {
  background: #fee;
  color: #c33;
  padding: 0.5rem 0.75rem;
  border-radius: 6px;
  font-size: 0.85rem;
  margin-bottom: 1rem;
}

@media (prefers-color-scheme: dark) {
  .error {
    background: #3a1a1a;
  }
}

.actions {
  display: flex;
  gap: 0.75rem;
  margin-bottom: 1.5rem;
}

.btn-primary {
  flex: 1;
  padding: 0.75rem;
  background: #4a6cf7;
  color: white;
  border: none;
  border-radius: 8px;
  font-size: 0.9rem;
  font-weight: 500;
  cursor: pointer;
  transition: background 0.2s;
}

.btn-primary:hover:not(:disabled) {
  background: #3a5ce5;
}

.btn-primary:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.btn-secondary {
  flex: 1;
  padding: 0.75rem;
  background: white;
  color: #4a6cf7;
  border: 1px solid #4a6cf7;
  border-radius: 8px;
  font-size: 0.9rem;
  font-weight: 500;
  cursor: pointer;
  transition: background 0.2s;
}

.btn-secondary:hover:not(:disabled) {
  background: #f0f4ff;
}

.btn-secondary:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

@media (prefers-color-scheme: dark) {
  .btn-secondary {
    background: #252540;
  }
  .btn-secondary:hover:not(:disabled) {
    background: #2f2f50;
  }
}

.loading {
  text-align: center;
  color: #888;
  padding: 1rem 0;
}

.sensitive-section {
  background: white;
  border-radius: 10px;
  padding: 1rem;
  box-shadow: 0 1px 6px rgba(0, 0, 0, 0.06);
}

@media (prefers-color-scheme: dark) {
  .sensitive-section {
    background: #252540;
  }
}

.field {
  margin-bottom: 1rem;
}

.field:last-of-type {
  margin-bottom: 0.5rem;
}

label {
  display: block;
  font-size: 0.75rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: #888;
  margin-bottom: 0.25rem;
}

.password-display {
  font-family: "SF Mono", "Fira Code", monospace;
  font-size: 1.1rem;
  padding: 0.5rem;
  background: #f0f4ff;
  border-radius: 6px;
  word-break: break-all;
  user-select: all;
}

@media (prefers-color-scheme: dark) {
  .password-display {
    background: #1a1a3e;
  }
}

.notes-display {
  font-size: 0.85rem;
  padding: 0.5rem;
  background: #fafafa;
  border-radius: 6px;
  white-space: pre-wrap;
  word-break: break-all;
  font-family: inherit;
  user-select: text;
}

@media (prefers-color-scheme: dark) {
  .notes-display {
    background: #1a1a2e;
  }
}

.auto-clear-hint {
  text-align: center;
  font-size: 0.75rem;
  color: #888;
  margin-top: 0.75rem;
}
</style>
