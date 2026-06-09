<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount } from "vue";
import { useRouter } from "vue-router";
import { invoke } from "@tauri-apps/api/core";
import type { Entry, PullResult, AppError } from "../types";
import { formatRelativeTime } from "../utils/format";
import { filterEntries } from "../utils/filter";

const router = useRouter();

const entries = ref<Entry[]>([]);
const search = ref("");
const loading = ref(false);
const pulling = ref(false);
const error = ref("");
const pullResult = ref("");
const toast = ref("");
let toastTimer: ReturnType<typeof setTimeout> | null = null;

const lastSyncTime = ref<number | null>(null);
const now = ref(Date.now());
let tickTimer: ReturnType<typeof setInterval> | null = null;

const lastSyncLabel = computed(() => {
  if (!lastSyncTime.value) return null;
  return formatRelativeTime(now.value, lastSyncTime.value);
});

const filteredEntries = () => {
  return filterEntries(entries.value, search.value);
};

async function loadEntries() {
  loading.value = true;
  error.value = "";
  try {
    entries.value = await invoke<Entry[]>("list_entries");
    lastSyncTime.value = Date.now();
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Failed to load entries";
  } finally {
    loading.value = false;
  }
}

async function pullRepo() {
  pulling.value = true;
  pullResult.value = "";
  error.value = "";
  try {
    const result = await invoke<PullResult>("pull_repo");
    if (result.changed) {
      pullResult.value = `Updated to ${result.head}`;
      await loadEntries();
      lastSyncTime.value = Date.now();
    } else {
      pullResult.value = "Already up to date";
    }
    // Clear pull result after 3 seconds
    setTimeout(() => {
      pullResult.value = "";
    }, 3000);
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Pull failed";
  } finally {
    pulling.value = false;
  }
}

function showToast(message: string) {
  toast.value = message;
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    toast.value = "";
    toastTimer = null;
  }, 3000);
}

async function copyPassword(entry: Entry) {
  try {
    const result = await invoke<import("../types").CopyResult>(
      "copy_password",
      {
        entryPath: entry.path,
      },
    );
    showToast(
      `✓ Copied ${result.entry_name} (${result.cleared_after_secs}s auto-clear)`,
    );
  } catch (e) {
    const appError = e as AppError;
    showToast(`Failed: ${appError?.message || "Copy failed"}`);
  }
}

function openEntry(entry: Entry) {
  router.push({ name: "entry", params: { pathMatch: entry.path } });
}

function openSettings() {
  router.push({ name: "settings" });
}

onMounted(() => {
  loadEntries();
  tickTimer = setInterval(() => {
    now.value = Date.now();
  }, 60_000);
});

onBeforeUnmount(() => {
  if (tickTimer) {
    clearInterval(tickTimer);
    tickTimer = null;
  }
});
</script>

<template>
  <main class="max-w-[480px] md:max-w-[600px] mx-auto p-4" role="main">
    <header class="flex justify-between items-center mb-4" role="banner">
      <h1 class="text-xl">🔐 gpm</h1>
      <div class="flex gap-2">
        <button
          @click="pullRepo"
          :disabled="pulling"
          class="btn-sm"
          aria-label="Pull updates"
          title="Pull updates"
        >
          <span aria-hidden="true">{{ pulling ? "⏳" : "↓" }}</span> Pull
        </button>
        <button
          @click="openSettings"
          class="btn-sm"
          aria-label="Settings"
          title="Settings"
        >
          <span aria-hidden="true">⚙</span>
        </button>
      </div>
    </header>

    <div
      v-if="lastSyncLabel"
      class="text-xs text-subtle text-center mb-2"
      aria-live="polite"
      role="status"
    >
      Last synced {{ lastSyncLabel }}
    </div>

    <div class="mb-4">
      <input
        v-model="search"
        type="search"
        placeholder="Search entries..."
        class="input-base w-full"
      />
    </div>

    <div
      v-if="error"
      class="bg-danger-soft text-danger p-2 px-3 rounded-sm text-sm mb-3 flex justify-between items-center"
      role="alert"
    >
      {{ error }}
      <button @click="loadEntries" class="btn-retry">Retry</button>
    </div>
    <div
      v-if="pullResult"
      class="bg-info-soft text-info p-2 px-3 rounded-sm text-sm mb-3"
      role="status"
      aria-live="polite"
    >
      {{ pullResult }}
    </div>
    <div
      v-if="toast"
      class="bg-success-soft text-success p-2 px-3 rounded-sm text-sm mb-3"
      role="status"
      aria-live="polite"
    >
      {{ toast }}
    </div>

    <div v-if="loading" class="text-center text-muted py-8">
      <span class="spinner"></span>
      <span>Loading entries...</span>
    </div>
    <div
      v-else-if="entries.length === 0 && !error"
      class="text-center text-muted py-8"
    >
      <span class="text-4xl block mb-2">🔒</span>
      <p>No passwords yet</p>
      <p class="text-xs text-subtle mt-1">
        Pull updates or check your repository
      </p>
    </div>
    <div
      v-else-if="filteredEntries().length === 0"
      class="text-center text-muted py-8"
    >
      <span class="text-4xl block mb-2">🔍</span>
      <p>No matches for "{{ search }}"</p>
    </div>

    <ul v-else class="list-none flex flex-col gap-0.5" role="list">
      <li
        v-for="entry in filteredEntries()"
        :key="entry.path"
        class="flex items-center justify-between p-[0.6rem_0.75rem] md:p-[0.8rem_1rem] bg-surface rounded-md transition-colors duration-150 min-h-12 hover:bg-hover"
      >
        <div
          class="flex-1 cursor-pointer min-w-0"
          tabindex="0"
          role="button"
          @click="openEntry(entry)"
          @keydown.enter="openEntry(entry)"
        >
          <span
            class="block font-medium whitespace-nowrap overflow-hidden text-ellipsis"
            >{{ entry.name }}</span
          >
          <span
            class="block text-xs text-muted whitespace-nowrap overflow-hidden text-ellipsis"
            >{{ entry.path }}</span
          >
        </div>
        <button
          @click.stop="copyPassword(entry)"
          class="bg-transparent border-none text-lg cursor-pointer p-1 px-[0.4rem] rounded-sm transition-colors duration-150 shrink-0 min-w-12 min-h-12 flex items-center justify-center hover:bg-[rgba(0,0,0,0.05)]"
          aria-label="Copy password"
          title="Copy password"
        >
          <span aria-hidden="true">📋</span>
        </button>
      </li>
    </ul>
  </main>
</template>

<style scoped>
.input-base {
  padding: 0.6rem 0.75rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  font-size: var(--text-base);
  background: var(--color-surface);
  color: inherit;
  min-height: 48px;
}

.input-base:focus {
  outline: none;
  border-color: var(--color-accent);
  box-shadow: 0 0 0 2px var(--color-accent-ring);
}

.btn-sm {
  padding: 0.3rem 0.6rem;
  font-size: var(--text-xs);
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-sm);
  background: var(--color-surface);
  color: inherit;
  cursor: pointer;
  min-height: 48px;
}

.btn-sm:hover {
  background: var(--color-hover);
}

.btn-sm:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.btn-retry {
  background: none;
  border: 1px solid var(--color-danger);
  color: var(--color-danger);
  padding: 0.15rem 0.5rem;
  border-radius: 4px;
  font-size: var(--text-xs);
  cursor: pointer;
  min-height: 48px;
}

.btn-retry:hover {
  opacity: 0.8;
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
