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

async function copyPassword(entry: Entry) {
  error.value = "";
  try {
    const result = await invoke<import("../types").CopyResult>(
      "copy_password",
      {
        entryPath: entry.path,
      },
    );
    // Show brief toast-like feedback
    pullResult.value = `Copied ${result.entry_name} (${result.cleared_after_secs}s)`;
    setTimeout(() => {
      pullResult.value = "";
    }, 3000);
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Copy failed";
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

    <div v-if="error" class="error">{{ error }}</div>
    <div v-if="pullResult" class="info">{{ pullResult }}</div>

    <div v-if="loading" class="empty">Loading entries...</div>
    <div v-else-if="filteredEntries().length === 0" class="empty">
      {{ entries.length === 0 ? "No entries found" : "No matches" }}
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
  max-width: 480px;
  margin: 0 auto;
  padding: 1rem;
}

.header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 1rem;
}

h1 {
  font-size: 1.25rem;
}

.header-actions {
  display: flex;
  gap: 0.5rem;
}

.search-bar {
  margin-bottom: 1rem;
}

.search-input {
  width: 100%;
  padding: 0.6rem 0.75rem;
  border: 1px solid #ddd;
  border-radius: 8px;
  font-size: 0.9rem;
  background: white;
  color: inherit;
}

.search-input:focus {
  outline: none;
  border-color: #4a6cf7;
  box-shadow: 0 0 0 2px rgba(74, 108, 247, 0.2);
}

@media (prefers-color-scheme: dark) {
  .search-input {
    background: #252540;
    border-color: #444;
  }
}

.error {
  background: #fee;
  color: #c33;
  padding: 0.5rem 0.75rem;
  border-radius: 6px;
  font-size: 0.85rem;
  margin-bottom: 0.75rem;
}

@media (prefers-color-scheme: dark) {
  .error {
    background: #3a1a1a;
  }
}

.info {
  background: #e8f4fd;
  color: #1976d2;
  padding: 0.5rem 0.75rem;
  border-radius: 6px;
  font-size: 0.85rem;
  margin-bottom: 0.75rem;
}

@media (prefers-color-scheme: dark) {
  .info {
    background: #1a2a3a;
  }
}

.empty {
  text-align: center;
  color: #888;
  padding: 2rem 0;
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
  padding: 0.6rem 0.75rem;
  background: white;
  border-radius: 8px;
  transition: background 0.15s;
}

.entry-item:hover {
  background: #f0f0f5;
}

@media (prefers-color-scheme: dark) {
  .entry-item {
    background: #252540;
  }
  .entry-item:hover {
    background: #2f2f50;
  }
}

.entry-info {
  flex: 1;
  cursor: pointer;
  min-width: 0;
}

.entry-name {
  display: block;
  font-weight: 500;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.entry-path {
  display: block;
  font-size: 0.75rem;
  color: #888;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.btn-copy {
  background: none;
  border: none;
  font-size: 1.1rem;
  cursor: pointer;
  padding: 0.25rem 0.4rem;
  border-radius: 6px;
  transition: background 0.15s;
  flex-shrink: 0;
}

.btn-copy:hover {
  background: rgba(0, 0, 0, 0.05);
}

.btn-sm {
  padding: 0.3rem 0.6rem;
  font-size: 0.8rem;
  border: 1px solid #ddd;
  border-radius: 6px;
  background: white;
  color: inherit;
  cursor: pointer;
}

.btn-sm:hover {
  background: #f0f0f5;
}

.btn-sm:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.btn-danger {
  border-color: #fcc;
  color: #c66;
}

@media (prefers-color-scheme: dark) {
  .btn-sm {
    background: #252540;
    border-color: #444;
  }
  .btn-sm:hover {
    background: #2f2f50;
  }
}
</style>
