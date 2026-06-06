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
const toast = ref("");
let autoHideTimer: ReturnType<typeof setTimeout> | null = null;
let toastTimer: ReturnType<typeof setTimeout> | null = null;

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
    revealed.value = false;
    notes.value = null;
    password.value = null;
    toast.value = `✓ Copied ${result.entry_name} (${result.cleared_after_secs}s auto-clear)`;
    if (toastTimer) clearTimeout(toastTimer);
    toastTimer = setTimeout(() => {
      toast.value = "";
      toastTimer = null;
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

function handleKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") {
    goBack();
  }
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
  <main class="entry-detail-page" role="main" @keydown="handleKeydown">
    <header class="header" role="banner">
      <button @click="goBack" class="btn-back" aria-label="Back to entry list">
        ← Back
      </button>
      <h1 class="entry-title">{{ entryName }}</h1>
    </header>

    <div v-if="error" class="error" role="alert">
      {{ error }}
      <span v-if="error.includes('ecrypt')" class="error-hint">
        Check your age identity and try again
      </span>
    </div>
    <div v-if="toast" class="toast" role="status" aria-live="polite">
      {{ toast }}
    </div>

    <div class="actions">
      <button
        @click="copyPassword"
        class="btn-primary"
        :disabled="loading"
        aria-label="Copy password to clipboard"
      >
        <span aria-hidden="true">📋</span> Copy Password
      </button>
      <button
        @click="showPassword"
        class="btn-secondary"
        :disabled="loading"
        :aria-label="revealed ? 'Password is showing' : 'Show password'"
      >
        <span aria-hidden="true">{{ revealed ? "👁" : "👁" }}</span>
        {{ revealed ? "Showing..." : "Show Password" }}
      </button>
    </div>

    <div v-if="loading" class="loading">
      <div class="spinner"></div>
      <span>Decrypting...</span>
    </div>

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
  </main>
</template>

<style scoped>
.entry-detail-page {
  max-width: var(--max-width);
  margin: 0 auto;
  padding: var(--screen-padding);
}

.header {
  display: flex;
  align-items: center;
  gap: var(--space-md);
  margin-bottom: var(--space-xl);
}

.btn-back {
  background: none;
  border: none;
  font-size: var(--font-size-md);
  cursor: pointer;
  color: var(--accent);
  padding: var(--space-xs);
  min-width: var(--touch-min);
  min-height: var(--touch-min);
}

.entry-title {
  font-size: var(--font-size-lg);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1;
}

.error {
  background: var(--danger-bg);
  color: var(--danger);
  padding: var(--space-sm) var(--space-md);
  border-radius: var(--radius-sm);
  font-size: var(--font-size-sm);
  margin-bottom: var(--space-lg);
}

.error-hint {
  display: block;
  font-size: var(--font-size-xs);
  opacity: 0.8;
  margin-top: var(--space-xs);
}

.toast {
  background: var(--success-bg);
  color: var(--success);
  padding: var(--space-sm) var(--space-md);
  border-radius: var(--radius-sm);
  font-size: var(--font-size-sm);
  margin-bottom: var(--space-lg);
}

.actions {
  display: flex;
  gap: var(--space-md);
  margin-bottom: var(--space-xl);
}

.btn-primary {
  flex: 1;
  padding: var(--space-md);
  background: var(--accent);
  color: white;
  border: none;
  border-radius: var(--radius-md);
  font-size: var(--font-size-base);
  font-weight: var(--font-weight-medium);
  cursor: pointer;
  transition: background 0.2s;
  min-height: var(--btn-min-height);
}

.btn-primary:hover:not(:disabled) {
  background: var(--accent-hover);
}

.btn-primary:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.btn-secondary {
  flex: 1;
  padding: var(--space-md);
  background: var(--bg-surface);
  color: var(--accent);
  border: 1px solid var(--accent);
  border-radius: var(--radius-md);
  font-size: var(--font-size-base);
  font-weight: var(--font-weight-medium);
  cursor: pointer;
  transition: background 0.2s;
  min-height: var(--btn-min-height);
}

.btn-secondary:hover:not(:disabled) {
  background: var(--bg-hover);
}

.btn-secondary:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.loading {
  text-align: center;
  color: var(--text-secondary);
  padding: var(--space-lg) 0;
}

.spinner {
  display: inline-block;
  width: 18px;
  height: 18px;
  border: 2px solid var(--border);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: spin 0.6s linear infinite;
  margin-right: var(--space-sm);
  vertical-align: middle;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

.sensitive-section {
  background: var(--bg-surface);
  border-radius: var(--radius-lg);
  padding: var(--space-lg);
  box-shadow: 0 1px 6px rgba(0, 0, 0, 0.06);
}

.field {
  margin-bottom: var(--space-lg);
}

.field:last-of-type {
  margin-bottom: var(--space-sm);
}

label {
  display: block;
  font-size: var(--font-size-xs);
  font-weight: var(--font-weight-semibold);
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--text-secondary);
  margin-bottom: var(--space-xs);
}

.password-display {
  font-family: var(--font-mono);
  font-size: var(--font-size-lg);
  padding: var(--space-sm);
  background: var(--accent-focus-ring);
  border-radius: var(--radius-sm);
  word-break: break-all;
  user-select: all;
}

.notes-display {
  font-size: var(--font-size-sm);
  padding: var(--space-sm);
  background: var(--bg-input);
  border-radius: var(--radius-sm);
  white-space: pre-wrap;
  word-break: break-all;
  font-family: inherit;
  user-select: text;
  max-height: 200px;
  overflow-y: auto;
}

.auto-clear-hint {
  text-align: center;
  font-size: var(--font-size-xs);
  color: var(--text-secondary);
  margin-top: var(--space-md);
}
</style>
