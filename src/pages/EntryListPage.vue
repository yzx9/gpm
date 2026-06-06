<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount } from "vue";
import { useRouter } from "vue-router";
import { invoke } from "@tauri-apps/api/core";
import type { Entry, PullResult, AppError } from "../types";

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

function formatRelativeTime(timestamp: number): string {
  const seconds = Math.floor((now.value - timestamp) / 1000);
  if (seconds < 60) return "just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ago`;
}

const lastSyncLabel = computed(() => {
  if (!lastSyncTime.value) return null;
  return formatRelativeTime(lastSyncTime.value);
});

const filteredEntries = () => {
  const q = search.value.toLowerCase();
  if (!q) return entries.value;
  return entries.value.filter(
    (e) => e.name.toLowerCase().includes(q) || e.path.toLowerCase().includes(q),
  );
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

async function resetConfig() {
  if (!confirm("Reset gpm? This will remove all local data and configuration."))
    return;
  try {
    await invoke("reset_config");
    router.push({ name: "setup" });
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Reset failed";
  }
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
  <main class="entry-list-page" role="main">
    <header class="header" role="banner">
      <h1>🔐 gpm</h1>
      <div class="header-actions">
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
          @click="resetConfig"
          class="btn-sm btn-danger"
          aria-label="Reset configuration"
          title="Reset configuration"
        >
          <span aria-hidden="true">⚙</span> Reset
        </button>
      </div>
    </header>

    <div
      v-if="lastSyncLabel"
      class="freshness"
      aria-live="polite"
      role="status"
    >
      Last synced {{ lastSyncLabel }}
    </div>

    <div class="search-bar">
      <input
        v-model="search"
        type="search"
        placeholder="Search entries..."
        class="search-input"
      />
    </div>

    <div v-if="error" class="error" role="alert">
      {{ error }}
      <button @click="loadEntries" class="btn-retry">Retry</button>
    </div>
    <div v-if="pullResult" class="info" role="status" aria-live="polite">
      {{ pullResult }}
    </div>
    <div v-if="toast" class="toast" role="status" aria-live="polite">
      {{ toast }}
    </div>

    <div v-if="loading" class="empty">
      <div class="spinner"></div>
      <span>Loading entries...</span>
    </div>
    <div v-else-if="entries.length === 0 && !error" class="empty">
      <span class="empty-icon">🔒</span>
      <p>No passwords yet</p>
      <p class="empty-hint">Pull updates or check your repository</p>
    </div>
    <div v-else-if="filteredEntries().length === 0" class="empty">
      <span class="empty-icon">🔍</span>
      <p>No matches for "{{ search }}"</p>
    </div>

    <ul v-else class="entry-list" role="list">
      <li
        v-for="entry in filteredEntries()"
        :key="entry.path"
        class="entry-item"
      >
        <div
          class="entry-info"
          tabindex="0"
          role="button"
          @click="openEntry(entry)"
          @keydown.enter="openEntry(entry)"
        >
          <span class="entry-name">{{ entry.name }}</span>
          <span class="entry-path">{{ entry.path }}</span>
        </div>
        <button
          @click.stop="copyPassword(entry)"
          class="btn-copy"
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
.entry-list-page {
  max-width: var(--max-width);
  margin: 0 auto;
  padding: var(--screen-padding);
}

.header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: var(--space-lg);
}

.freshness {
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
  text-align: center;
  margin-bottom: var(--space-sm);
}

h1 {
  font-size: var(--font-size-xl);
}

.header-actions {
  display: flex;
  gap: var(--space-sm);
}

.search-bar {
  margin-bottom: var(--space-lg);
}

.search-input {
  width: 100%;
  padding: 0.6rem var(--space-md);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  font-size: var(--font-size-base);
  background: var(--bg-surface);
  color: inherit;
  min-height: var(--input-min-height);
}

.search-input:focus {
  outline: none;
  border-color: var(--accent);
  box-shadow: 0 0 0 2px var(--accent-focus-ring);
}

.error {
  background: var(--danger-bg);
  color: var(--danger);
  padding: var(--space-sm) var(--space-md);
  border-radius: var(--radius-sm);
  font-size: var(--font-size-sm);
  margin-bottom: var(--space-md);
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.btn-retry {
  background: none;
  border: 1px solid var(--danger);
  color: var(--danger);
  padding: 0.15rem var(--space-sm);
  border-radius: 4px;
  font-size: var(--font-size-xs);
  cursor: pointer;
  min-height: var(--btn-min-height);
}

.btn-retry:hover {
  opacity: 0.8;
}

.info {
  background: var(--info-bg);
  color: var(--info);
  padding: var(--space-sm) var(--space-md);
  border-radius: var(--radius-sm);
  font-size: var(--font-size-sm);
  margin-bottom: var(--space-md);
}

.toast {
  background: var(--success-bg);
  color: var(--success);
  padding: var(--space-sm) var(--space-md);
  border-radius: var(--radius-sm);
  font-size: var(--font-size-sm);
  margin-bottom: var(--space-md);
}

.empty {
  text-align: center;
  color: var(--text-secondary);
  padding: var(--space-2xl) 0;
}

.empty-icon {
  font-size: 2rem;
  display: block;
  margin-bottom: var(--space-sm);
}

.empty-hint {
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
  margin-top: var(--space-xs);
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

.entry-list {
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.entry-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.6rem var(--space-md);
  background: var(--bg-surface);
  border-radius: var(--radius-md);
  transition: background 0.15s;
  min-height: var(--touch-min);
}

.entry-item:hover {
  background: var(--bg-hover);
}

.entry-info {
  flex: 1;
  cursor: pointer;
  min-width: 0;
}

.entry-name {
  display: block;
  font-weight: var(--font-weight-medium);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.entry-path {
  display: block;
  font-size: var(--font-size-xs);
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.btn-copy {
  background: none;
  border: none;
  font-size: var(--font-size-lg);
  cursor: pointer;
  padding: var(--space-xs) 0.4rem;
  border-radius: var(--radius-sm);
  transition: background 0.15s;
  flex-shrink: 0;
  min-width: var(--touch-min);
  min-height: var(--touch-min);
  display: flex;
  align-items: center;
  justify-content: center;
}

.btn-copy:hover {
  background: rgba(0, 0, 0, 0.05);
}

.btn-sm {
  padding: 0.3rem 0.6rem;
  font-size: var(--font-size-xs);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-surface);
  color: inherit;
  cursor: pointer;
  min-height: var(--btn-min-height);
}

.btn-sm:hover {
  background: var(--bg-hover);
}

.btn-sm:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.btn-danger {
  border-color: var(--danger-border);
  color: #c66;
}

@media (min-width: 768px) {
  .entry-list-page {
    max-width: 600px;
  }

  .entry-item {
    padding: 0.8rem var(--space-lg);
  }
}
</style>
