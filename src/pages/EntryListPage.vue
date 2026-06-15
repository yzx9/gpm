<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount } from "vue";
import { useRouter } from "vue-router";
import { invoke } from "@tauri-apps/api/core";
import type {
  AppError,
  AuthenticityState,
  CommitSigInfo,
  Entry,
  PullResult,
} from "../types";
import { formatRelativeTime } from "../utils/format";
import { filterEntries } from "../utils/filter";
import { statusGlyph, statusLabel } from "../utils/signature";

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

// ── Authenticity (badge + pull modals) ───────────────────────────────────
const authState = ref<AuthenticityState | null>(null);
/** Audit-mode open issues from the last pull → drives the mismatch modal. */
const auditIssues = ref<CommitSigInfo[] | null>(null);
/** Enforce-block result from the last pull → drives the block modal. */
const blockIssues = ref<CommitSigInfo[] | null>(null);

/** The indicator badge for the current authenticity state. */
const badge = computed<{ glyph: string; cls: string; title: string }>(() => {
  const s = authState.value;
  if (!s || s.mode === "off") {
    return {
      glyph: "⚪",
      cls: "badge-off",
      title: "Signature verification off",
    };
  }
  switch (s.head_status.kind) {
    case "verified":
      return {
        glyph: "✓",
        cls: "badge-ok",
        title: "HEAD signed by a trusted key",
      };
    case "unknown":
      return {
        glyph: "—",
        cls: "badge-none",
        title: "Signature not checked yet",
      };
    default:
      return {
        glyph: "⚠",
        cls: "badge-warn",
        title: `${statusLabel(s.head_status)} — tap to review`,
      };
  }
});

const lastSyncLabel = computed(() => {
  if (!lastSyncTime.value) return null;
  return formatRelativeTime(now.value, lastSyncTime.value);
});

const filteredEntries = () => {
  return filterEntries(entries.value, search.value);
};

async function loadAuthState() {
  try {
    authState.value = await invoke<AuthenticityState>("get_authenticity_state");
  } catch {
    // Verification unavailable (e.g. repo mid-clone) — leave the badge as-is.
  }
}

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
  auditIssues.value = null;
  blockIssues.value = null;
  try {
    const result = await invoke<PullResult>("pull_repo");
    if (result.changed) {
      pullResult.value = `Updated to ${result.head}`;
      await loadEntries();
      lastSyncTime.value = Date.now();
    } else {
      pullResult.value = "Already up to date";
    }
    // Refresh the badge with the new HEAD state.
    await loadAuthState();

    // Audit mismatch → informational modal (pull already succeeded).
    if (
      result.authenticity.mode === "audit" &&
      result.authenticity.open_issues.length > 0
    ) {
      auditIssues.value = result.authenticity.open_issues;
    }
    // Enforce block → HEAD did not advance; explain + offer actions.
    if (result.authenticity.blocked) {
      blockIssues.value = result.authenticity.open_issues;
    }

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

async function ignoreIssue(commit: CommitSigInfo) {
  try {
    await invoke("ignore_commit_issue", { commit: commit.hash });
    showToast("Ignored this commit's issue");
    // Remove it from the modal list.
    if (auditIssues.value) {
      auditIssues.value = auditIssues.value.filter(
        (c) => c.hash !== commit.hash,
      );
      if (auditIssues.value.length === 0) auditIssues.value = null;
    }
  } catch (e) {
    const appError = e as AppError;
    showToast(appError?.message || "Failed to ignore");
  }
}

async function trustBlockSigner(commit: CommitSigInfo) {
  const label = window.prompt(
    "Trust this signer? Enter a label:",
    commit.short_hash,
  );
  if (label === null) return;
  try {
    await invoke("trust_commit_signer", {
      commit: commit.hash,
      label: label.trim() || commit.short_hash,
    });
    showToast("✓ Signer trusted — pull again");
    blockIssues.value = null;
    await loadAuthState();
  } catch (e) {
    const appError = e as AppError;
    showToast(appError?.message || "Failed to trust signer");
  }
}

async function switchToAudit() {
  try {
    await invoke("set_verification_mode", { mode: "audit" });
    showToast("Switched to Audit — pull again");
    blockIssues.value = null;
    await loadAuthState();
  } catch (e) {
    const appError = e as AppError;
    showToast(appError?.message || "Failed to switch mode");
  }
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

function openHistory() {
  router.push({ name: "history" });
}

onMounted(() => {
  loadEntries();
  loadAuthState();
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
      <div class="flex gap-2 items-center">
        <button
          @click="openHistory"
          class="badge-btn"
          :class="badge.cls"
          :aria-label="badge.title"
          :title="badge.title"
        >
          <span aria-hidden="true">{{ badge.glyph }}</span>
        </button>
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

    <!-- Audit-mode mismatch modal (pull succeeded; informational) -->
    <div
      v-if="auditIssues"
      class="fixed inset-0 bg-black/40 z-40 flex items-end sm:items-center justify-center p-4"
      role="dialog"
      aria-modal="true"
      aria-label="Signature check"
    >
      <div class="modal-card w-full max-w-[480px]">
        <h2 class="text-base font-medium mb-1">Signature check</h2>
        <p class="text-xs text-muted mb-3">
          Pulled {{ auditIssues.length }}
          {{ auditIssues.length === 1 ? "commit has" : "commits have" }} a
          signature issue:
        </p>
        <ul class="flex flex-col gap-2 mb-3">
          <li
            v-for="c in auditIssues"
            :key="c.hash"
            class="flex items-center gap-2 text-sm"
          >
            <span class="text-lg" aria-hidden="true">{{
              statusGlyph(c.status)
            }}</span>
            <code class="text-xs text-muted">{{ c.short_hash }}</code>
            <span class="flex-1 truncate">{{ c.subject }}</span>
            <span class="text-xs text-muted">{{ statusLabel(c.status) }}</span>
          </li>
        </ul>
        <div class="flex gap-2">
          <button class="btn-sm flex-1" @click="openHistory">
            Review in history
          </button>
          <button
            v-if="auditIssues.length === 1"
            class="btn-sm flex-1"
            @click="ignoreIssue(auditIssues[0]!)"
          >
            Ignore this commit
          </button>
          <button class="btn-sm flex-1" @click="auditIssues = null">
            Dismiss
          </button>
        </div>
      </div>
    </div>

    <!-- Enforce-block modal (HEAD did not advance) -->
    <div
      v-if="blockIssues"
      class="fixed inset-0 bg-black/40 z-40 flex items-end sm:items-center justify-center p-4"
      role="dialog"
      aria-modal="true"
      aria-label="Pull blocked"
    >
      <div class="modal-card w-full max-w-[480px]">
        <h2 class="text-base font-medium mb-1 text-danger">Pull blocked</h2>
        <p class="text-xs text-muted mb-3">
          Enforce mode refused to update the store — HEAD did not advance.
          Resolve the signature issue, then pull again.
        </p>
        <ul class="flex flex-col gap-2 mb-3">
          <li
            v-for="c in blockIssues"
            :key="c.hash"
            class="flex items-center gap-2 text-sm"
          >
            <span class="text-lg" aria-hidden="true">{{
              statusGlyph(c.status)
            }}</span>
            <code class="text-xs text-muted">{{ c.short_hash }}</code>
            <span class="flex-1 truncate">{{ c.subject }}</span>
            <span class="text-xs text-muted">{{ statusLabel(c.status) }}</span>
          </li>
        </ul>
        <div class="flex flex-col gap-2">
          <button
            v-if="blockIssues.some((c) => c.status.kind === 'untrusted_key')"
            class="btn-sm"
            @click="
              trustBlockSigner(
                blockIssues.find((c) => c.status.kind === 'untrusted_key')!,
              )
            "
          >
            Trust this signer
          </button>
          <button class="btn-sm" @click="switchToAudit">
            Switch to Audit mode
          </button>
          <button class="btn-sm" @click="blockIssues = null">Cancel</button>
        </div>
      </div>
    </div>
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

.badge-btn {
  width: 36px;
  height: 36px;
  min-height: 36px;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-sm);
  background: var(--color-surface);
  cursor: pointer;
  font-size: 1rem;
  display: flex;
  align-items: center;
  justify-content: center;
}
.badge-btn:hover {
  background: var(--color-hover);
}
.badge-ok {
  color: var(--color-success, #3a9);
}
.badge-warn {
  color: var(--color-warning, #c93);
}
.badge-off,
.badge-none {
  color: var(--color-subtle, #999);
}

.modal-card {
  padding: 1rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  background: var(--color-surface);
}
</style>
