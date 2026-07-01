<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { invoke } from "@tauri-apps/api/core";
import type { AppError, CommitSigInfo } from "../types";
import { formatRelativeTime } from "../utils/format";
import {
  isIgnorable,
  signerFp,
  statusBgClass,
  statusClass,
  statusGlyph,
  statusLabel,
} from "../utils/signature";
import BaseButton from "../components/base/BaseButton.vue";
import BaseSpinner from "../components/base/BaseSpinner.vue";

const router = useRouter();

const commits = ref<CommitSigInfo[]>([]);
const loading = ref(false);
const error = ref("");
const toast = ref("");
let toastTimer: ReturnType<typeof setTimeout> | null = null;

const selected = ref<CommitSigInfo | null>(null);
const actionLoading = ref(false);

const now = ref(Date.now());
let tickTimer: ReturnType<typeof setInterval> | null = null;

const relativeNow = computed(() => now.value);

function showToast(message: string) {
  toast.value = message;
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    toast.value = "";
    toastTimer = null;
  }, 3000);
}

async function loadHistory() {
  loading.value = true;
  error.value = "";
  try {
    commits.value = await invoke<CommitSigInfo[]>("list_commit_signatures", {
      limit: 50,
    });
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Failed to load commit history";
  } finally {
    loading.value = false;
  }
}

function openDetail(commit: CommitSigInfo) {
  selected.value = commit;
}

function closeDetail() {
  selected.value = null;
}

async function onTrust(commit: CommitSigInfo) {
  const fp = signerFp(commit.status);
  const suggested = fp ? fp.replace("SHA256:", "").slice(0, 12) : "signer";
  const label = window.prompt("Label for this trusted signer:", suggested);
  if (label === null) return;
  actionLoading.value = true;
  try {
    await invoke("trust_commit_signer", {
      commit: commit.hash,
      label: label.trim() || suggested,
    });
    showToast(`✓ Trusted ${label || suggested}`);
    await loadHistory();
    selected.value = null;
  } catch (e) {
    const appError = e as AppError;
    showToast(appError?.message || "Failed to trust signer");
  } finally {
    actionLoading.value = false;
  }
}

async function onIgnore(commit: CommitSigInfo) {
  actionLoading.value = true;
  try {
    await invoke("ignore_commit_issue", { commit: commit.hash });
    showToast("Issue ignored for this commit");
    await loadHistory();
    selected.value = null;
  } catch (e) {
    const appError = e as AppError;
    showToast(appError?.message || "Failed to ignore");
  } finally {
    actionLoading.value = false;
  }
}

async function copyHash(commit: CommitSigInfo) {
  try {
    await navigator.clipboard.writeText(commit.hash);
    showToast("✓ Hash copied");
  } catch {
    showToast("Copy failed");
  }
}

function openSettings() {
  router.push({ name: "settings" });
}

onMounted(() => {
  loadHistory();
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
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <header class="flex justify-between items-center mb-4" role="banner">
      <h1 class="text-xl">📜 History</h1>
      <div class="flex gap-2">
        <BaseButton
          size="sm"
          :disabled="loading"
          @click="loadHistory"
          aria-label="Re-check signatures"
          title="Re-check signatures"
        >
          ⟳ Re-check
        </BaseButton>
        <BaseButton size="sm" @click="openSettings" aria-label="Settings">
          ⚙
        </BaseButton>
      </div>
    </header>

    <p class="text-xs text-muted mb-4">
      Recent commits and their SSH signature status. Tap a commit to review or
      trust its signer.
    </p>

    <div
      v-if="error"
      class="bg-danger-soft text-danger p-2 px-3 rounded-sm text-sm mb-3"
      role="alert"
    >
      {{ error }}
    </div>

    <div
      v-if="loading && commits.length === 0"
      class="flex items-center justify-center gap-2 text-center text-muted py-8"
    >
      <BaseSpinner />
      <span>Loading history...</span>
    </div>
    <div
      v-else-if="commits.length === 0 && !error"
      class="text-center text-muted py-8"
    >
      <span class="text-4xl block mb-2">📭</span>
      <p>No commits found</p>
    </div>

    <ul v-else class="list-none flex flex-col gap-0.5" role="list">
      <li
        v-for="commit in commits"
        :key="commit.hash"
        class="flex items-center gap-2 p-[0.6rem_0.75rem] bg-surface rounded-md min-h-12 cursor-pointer hover:bg-hover"
        role="button"
        tabindex="0"
        @click="openDetail(commit)"
        @keydown.enter="openDetail(commit)"
      >
        <span
          class="text-lg w-6 text-center shrink-0"
          :class="statusClass(commit.status)"
          aria-hidden="true"
          >{{ statusGlyph(commit.status) }}</span
        >
        <div class="flex-1 min-w-0">
          <div class="flex items-baseline gap-2">
            <code class="text-xs text-muted">{{ commit.short_hash }}</code>
            <span
              class="font-medium whitespace-nowrap overflow-hidden text-ellipsis"
              >{{ commit.subject || "(no message)" }}</span
            >
          </div>
          <div
            class="text-xs text-subtle whitespace-nowrap overflow-hidden text-ellipsis"
          >
            {{ commit.author }}
          </div>
        </div>
        <span class="text-xs text-subtle shrink-0">
          {{ formatRelativeTime(relativeNow, Date.parse(commit.date)) }}
        </span>
        <span
          v-if="commit.ignored"
          class="text-[0.6rem] text-subtle shrink-0 px-1 rounded-sm bg-edge"
          >ignored</span
        >
      </li>
    </ul>

    <!-- Detail sheet -->
    <div
      v-if="selected"
      class="fixed inset-0 bg-black/40 z-40 flex items-end sm:items-center justify-center p-4"
      role="dialog"
      aria-modal="true"
      @click.self="closeDetail"
    >
      <div class="settings-card w-full max-w-120">
        <div class="flex justify-between items-start mb-2">
          <code class="text-xs text-muted">{{ selected.short_hash }}</code>
          <button class="btn-copy" @click="closeDetail" aria-label="Close">
            ✕
          </button>
        </div>

        <h2 class="text-base font-medium wrap-break-word">
          {{ selected.subject || "(no message)" }}
        </h2>
        <p class="text-xs text-muted mt-1 wrap-break-word">
          {{ selected.author }}
        </p>
        <p class="text-xs text-subtle mt-0.5">{{ selected.date }}</p>

        <div
          class="mt-3 p-2 rounded-sm text-sm flex items-center gap-2"
          :class="statusBgClass(selected.status)"
        >
          <span class="text-lg" aria-hidden="true">{{
            statusGlyph(selected.status)
          }}</span>
          <div class="flex-1 min-w-0">
            <div class="font-medium">{{ statusLabel(selected.status) }}</div>
            <div
              v-if="signerFp(selected.status)"
              class="text-xs text-muted break-all"
            >
              {{ signerFp(selected.status) }}
            </div>
          </div>
          <span
            v-if="selected.ignored"
            class="text-[0.6rem] text-subtle px-1 rounded-sm bg-edge"
            >ignored</span
          >
        </div>

        <p
          v-if="selected.status.kind === 'bad_signature'"
          class="text-xs text-danger mt-2"
        >
          ⚠ This commit's signature does not validate — the commit object may
          have been altered after signing. It cannot be ignored in Enforce mode.
        </p>

        <div class="flex flex-col gap-2 mt-4">
          <BaseButton
            v-if="selected.status.kind === 'untrusted_key'"
            variant="action"
            :disabled="actionLoading"
            @click="onTrust(selected)"
          >
            Trust this signer
          </BaseButton>
          <BaseButton
            v-if="isIgnorable(selected.status) && !selected.ignored"
            variant="action"
            :disabled="actionLoading"
            @click="onIgnore(selected)"
          >
            Ignore this issue
          </BaseButton>
          <BaseButton variant="action" @click="copyHash(selected)">
            Copy hash
          </BaseButton>
        </div>
      </div>
    </div>

    <div
      v-if="toast"
      class="fixed bottom-4 left-1/2 -translate-x-1/2 bg-success-soft text-success p-2 px-4 rounded-md text-sm shadow-lg z-50"
      role="status"
      aria-live="polite"
    >
      {{ toast }}
    </div>
  </main>
</template>

<style scoped>
.settings-card {
  padding: 1rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  background: var(--color-surface);
}

.sig-verified {
  color: var(--color-success, #3a9);
}
.sig-warn {
  color: var(--color-warning, #c93);
}
.sig-bad {
  color: var(--color-danger, #c66);
}
</style>
