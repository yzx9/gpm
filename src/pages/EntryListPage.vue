<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { ref, onMounted } from "vue";
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

onMounted(loadEntries);
</script>

<template>
  <div class="entry-list-page">
    <header class="header">
      <h1>🔐 gpm</h1>
      <div class="header-actions">
        <button
          @click="pullRepo"
          :disabled="pulling"
          class="btn-sm"
          title="Pull updates"
        >
          {{ pulling ? "⏳" : "↓ Pull" }}
        </button>
        <button
          @click="resetConfig"
          class="btn-sm btn-danger"
          title="Reset configuration"
        >
          ⚙ Reset
        </button>
      </div>
    </header>

    <div class="search-bar">
      <input
        v-model="search"
        type="search"
        placeholder="Search entries..."
        class="search-input"
      />
    </div>

    <div v-if="error" class="error">
      {{ error }}
      <button @click="loadEntries" class="btn-retry">Retry</button>
    </div>
    <div v-if="pullResult" class="info">{{ pullResult }}</div>
    <div v-if="toast" class="toast">{{ toast }}</div>

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

    <ul v-else class="entry-list">
      <li
        v-for="entry in filteredEntries()"
        :key="entry.path"
        class="entry-item"
      >
        <div class="entry-info" @click="openEntry(entry)">
          <span class="entry-name">{{ entry.name }}</span>
          <span class="entry-path">{{ entry.path }}</span>
        </div>
        <button
          @click.stop="copyPassword(entry)"
          class="btn-copy"
          title="Copy password"
        >
          📋
        </button>
      </li>
    </ul>
  </div>
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
</style>
