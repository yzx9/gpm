<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { invoke } from "@tauri-apps/api/core";
import type { SensitiveContent, AppError } from "../types";
import { useSecretReveal } from "../utils/useSecretReveal";

const route = useRoute();
const router = useRouter();

const entryPath = decodeURIComponent(
  Array.isArray(route.params.pathMatch)
    ? route.params.pathMatch[0]
    : route.params.pathMatch,
);
const entryName = entryPath.replace(/\.age$/, "");

// Sensitive state lives in the shared secure-reveal composable: 30s auto-clear,
// wipe on unmount, wipe on browser back. `copyPassword` calls `clear()` itself.
const { password, notes, revealed, reveal, clear } = useSecretReveal();
const loading = ref(false);
const error = ref("");
const toast = ref("");
const deleting = ref(false);
let toastTimer: ReturnType<typeof setTimeout> | null = null;

function showToast(message: string) {
  toast.value = message;
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    toast.value = "";
    toastTimer = null;
  }, 3000);
}

async function showPassword() {
  loading.value = true;
  error.value = "";
  try {
    const result = await invoke<SensitiveContent>("show_password", {
      entryPath,
    });
    reveal(result);
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
    clear();
    showToast(
      `✓ Copied ${result.entry_name} (${result.cleared_after_secs}s auto-clear)`,
    );
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Copy failed";
  }
}

async function deleteSecret() {
  if (deleting.value) return;
  if (
    !confirm(
      `Delete ${entryName}? This removes it everywhere on the next sync. gpm has no in-app undo — recovery is only possible via git history with external tooling.`,
    )
  ) {
    return;
  }
  deleting.value = true;
  error.value = "";
  try {
    const result = await invoke<{ commit: string }>("delete_secret", {
      name: entryName,
    });
    clear();
    showToast(`✓ Deleted (commit ${result.commit})`);
    router.push({ name: "entries" });
  } catch (e) {
    const appError = e as AppError;
    if (appError?.code === "PUSH_REJECTED") {
      // Remote diverged — delete was rolled back. Defer to the sync flow.
      showToast("Remote moved — sync to review and re-delete.");
    } else {
      error.value = appError?.message || "Delete failed";
    }
  } finally {
    deleting.value = false;
  }
}

function goBack() {
  clear();
  router.push({ name: "entries" });
}

function handleKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") {
    goBack();
  }
}
</script>

<template>
  <main class="max-w-[480px] mx-auto p-4" role="main" @keydown="handleKeydown">
    <header class="flex items-center gap-3 mb-6" role="banner">
      <button
        @click="goBack"
        class="bg-transparent border-none text-base cursor-pointer text-accent p-1 min-w-12 min-h-12"
        aria-label="Back to entry list"
      >
        ← Back
      </button>
      <h1
        class="text-lg whitespace-nowrap overflow-hidden text-ellipsis flex-1"
      >
        {{ entryName }}
      </h1>
    </header>

    <div
      v-if="error"
      class="bg-danger-soft text-danger p-2 px-3 rounded-sm text-sm mb-4"
      role="alert"
    >
      {{ error }}
      <span
        v-if="error.includes('ecrypt')"
        class="block text-xs opacity-80 mt-1"
      >
        Check your age identity and try again
      </span>
    </div>
    <div
      v-if="toast"
      class="bg-success-soft text-success p-2 px-3 rounded-sm text-sm mb-4"
      role="status"
      aria-live="polite"
    >
      {{ toast }}
    </div>

    <div class="flex gap-3 mb-6">
      <button
        @click="copyPassword"
        class="btn-primary flex-1"
        :disabled="loading"
        aria-label="Copy password to clipboard"
      >
        <span aria-hidden="true">📋</span> Copy Password
      </button>
      <button
        @click="showPassword"
        class="btn-secondary flex-1"
        :disabled="loading"
        :aria-label="revealed ? 'Password is showing' : 'Show password'"
      >
        <span aria-hidden="true">{{ revealed ? "👁" : "👁" }}</span>
        {{ revealed ? "Showing..." : "Show Password" }}
      </button>
    </div>

    <button
      @click="deleteSecret"
      class="btn-danger w-full mb-6"
      :disabled="deleting || loading"
      :aria-label="`Delete ${entryName}`"
    >
      {{ deleting ? "Deleting…" : "Delete" }}
    </button>

    <div v-if="loading" class="text-center text-muted py-4">
      <span class="spinner"></span>
      <span>Decrypting...</span>
    </div>

    <div
      v-if="revealed && password !== null"
      class="bg-surface rounded-lg p-4 shadow-[0_1px_6px_rgba(0,0,0,0.06)]"
    >
      <div class="mb-4">
        <label
          class="block text-xs font-semibold uppercase tracking-wide text-muted mb-1"
          >Password</label
        >
        <div
          class="font-mono text-lg p-2 bg-accent-ring rounded-sm break-all select-all"
        >
          {{ password }}
        </div>
      </div>

      <div v-if="notes" class="mb-2">
        <label
          class="block text-xs font-semibold uppercase tracking-wide text-muted mb-1"
          >Notes</label
        >
        <pre
          class="text-sm p-2 bg-input rounded-sm whitespace-pre-wrap break-all font-[inherit] select-text max-h-[200px] overflow-y-auto"
          >{{ notes }}</pre
        >
      </div>

      <p class="text-center text-xs text-muted mt-3">
        Auto-clears in 30 seconds
      </p>
    </div>
  </main>
</template>

<style scoped>
.btn-primary {
  padding: 0.75rem;
  background: var(--color-accent);
  color: white;
  border: none;
  border-radius: var(--radius-md);
  font-size: var(--text-base);
  font-weight: 500;
  cursor: pointer;
  transition: background 0.2s;
  min-height: 48px;
}

.btn-primary:hover:not(:disabled) {
  background: var(--color-accent-deep);
}

.btn-primary:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.btn-secondary {
  padding: 0.75rem;
  background: var(--color-surface);
  color: var(--color-accent);
  border: 1px solid var(--color-accent);
  border-radius: var(--radius-md);
  font-size: var(--text-base);
  font-weight: 500;
  cursor: pointer;
  transition: background 0.2s;
  min-height: 48px;
}

.btn-secondary:hover:not(:disabled) {
  background: var(--color-hover);
}

.btn-secondary:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.btn-danger {
  padding: 0.75rem;
  background: var(--color-surface);
  color: var(--color-danger);
  border: 1px solid var(--color-danger);
  border-radius: var(--radius-md);
  font-size: var(--text-base);
  font-weight: 500;
  cursor: pointer;
  transition: background 0.2s;
  min-height: 48px;
}

.btn-danger:hover:not(:disabled) {
  background: var(--color-danger-soft);
}

.btn-danger:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.spinner {
  display: inline-block;
  width: 18px;
  height: 18px;
  border: 2px solid var(--color-edge);
  border-top-color: var(--color-accent);
  border-radius: 50%;
  animation: spin 0.6s linear infinite;
  margin-right: 0.5rem;
  vertical-align: middle;
}
</style>
