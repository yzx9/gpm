<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import type { AppError, CommitSigInfo } from "@/api";
import {
  ignoreCommitIssue,
  listCommitSignatures,
  trustCommitSigner,
} from "@/api";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseModalShell from "@/components/base/BaseModalShell.vue";
import BaseSpinner from "@/components/base/BaseSpinner.vue";
import CommitSigIndicator from "@/components/CommitSigIndicator.vue";
import { useToast } from "@/composables";
import { formatRelativeTime } from "@/utils/format";
import { isIgnorable, signerFp } from "@/utils/signature";
import {
  GitCommitHorizontal,
  History,
  RefreshCw,
  Settings,
  TriangleAlert,
  X,
} from "@lucide/vue";
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useRouter } from "vue-router";

const router = useRouter();
const { toast } = useToast();

const commits = ref<CommitSigInfo[]>([]);
const loading = ref(false);
const error = ref("");

const selected = ref<CommitSigInfo | null>(null);
const actionLoading = ref(false);

const now = ref(Date.now());
let tickTimer: ReturnType<typeof setInterval> | null = null;

const relativeNow = computed(() => now.value);

async function loadHistory() {
  loading.value = true;
  error.value = "";
  try {
    commits.value = await listCommitSignatures(50);
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
    await trustCommitSigner(commit.hash, label.trim() || suggested);
    toast.success(`✓ Trusted ${label || suggested}`);
    await loadHistory();
    selected.value = null;
  } catch (e) {
    const appError = e as AppError;
    toast.danger(appError?.message || "Failed to trust signer");
  } finally {
    actionLoading.value = false;
  }
}

async function onIgnore(commit: CommitSigInfo) {
  actionLoading.value = true;
  try {
    await ignoreCommitIssue(commit.hash);
    toast.success("Issue ignored for this commit");
    await loadHistory();
    selected.value = null;
  } catch (e) {
    const appError = e as AppError;
    toast.danger(appError?.message || "Failed to ignore");
  } finally {
    actionLoading.value = false;
  }
}

async function copyHash(commit: CommitSigInfo) {
  try {
    await navigator.clipboard.writeText(commit.hash);
    toast.success("✓ Hash copied");
  } catch {
    toast.danger("Copy failed");
  }
}

function openSettings() {
  // Forward nav, not a pop: History is reached from both Settings and the entry
  // list, so the Settings button must always go to Settings regardless of the
  // opener.
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
      <h1 class="text-xl flex items-center gap-1">
        <BaseIcon :icon="History" :size="24" /> History
      </h1>
      <div class="flex gap-2">
        <BaseButton
          size="sm"
          :disabled="loading"
          @click="loadHistory"
          aria-label="Re-check signatures"
          title="Re-check signatures"
        >
          <BaseIcon :icon="RefreshCw" /> Re-check
        </BaseButton>
        <BaseButton size="sm" @click="openSettings" aria-label="Settings">
          <BaseIcon :icon="Settings" />
        </BaseButton>
      </div>
    </header>

    <p class="text-xs text-muted mb-4">
      Recent commits and their SSH signature status. Tap a commit to review or
      trust its signer.
    </p>

    <BaseAlert v-if="error" variant="danger" class="mb-3">
      {{ error }}
    </BaseAlert>

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
      <BaseIcon
        :icon="GitCommitHorizontal"
        :size="40"
        class="block mb-2 mx-auto text-subtle"
      />
      <p>No commits found</p>
    </div>

    <ul v-else class="list-none flex flex-col gap-0.5" role="list">
      <li
        v-for="commit in commits"
        :key="commit.hash"
        class="flex items-start gap-2 p-[0.6rem_0.75rem] bg-surface rounded-md min-h-12 cursor-pointer hover:bg-hover active:bg-hover"
        role="button"
        tabindex="0"
        @click="openDetail(commit)"
        @keydown.enter="openDetail(commit)"
      >
        <CommitSigIndicator
          :status="commit.status"
          class="w-6 text-center shrink-0 mt-0.5"
        />
        <div class="flex-1 min-w-0 flex flex-col gap-0.5">
          <span class="font-medium wrap-break-word line-clamp-2">{{
            commit.subject || "(no message)"
          }}</span>
          <div class="flex items-center gap-1.5 text-xs text-subtle min-w-0">
            <code class="shrink-0">{{ commit.short_hash }}</code>
            <span aria-hidden="true" class="shrink-0">·</span>
            <span class="truncate min-w-0">{{ commit.author }}</span>
            <span aria-hidden="true" class="shrink-0">·</span>
            <span class="shrink-0">{{
              formatRelativeTime(relativeNow, Date.parse(commit.date))
            }}</span>
          </div>
        </div>
        <span
          v-if="commit.ignored"
          class="text-[0.6rem] text-subtle shrink-0 mt-0.5 px-1 rounded-sm bg-edge"
          >ignored</span
        >
      </li>
    </ul>

    <!-- Detail sheet -->
    <BaseModalShell
      v-if="selected"
      variant="sheet"
      aria-label="Commit detail"
      @close="closeDetail"
    >
      <div class="flex justify-between items-start mb-2">
        <code class="text-xs text-muted">{{ selected.short_hash }}</code>
        <button class="btn-copy" @click="closeDetail" aria-label="Close">
          <BaseIcon :icon="X" />
        </button>
      </div>

      <h2 class="text-base font-medium wrap-break-word">
        {{ selected.subject || "(no message)" }}
      </h2>
      <p class="text-xs text-muted mt-1 wrap-break-word">
        {{ selected.author }}
      </p>
      <p class="text-xs text-subtle mt-0.5">{{ selected.date }}</p>

      <CommitSigIndicator
        :status="selected.status"
        variant="banner"
        :ignored="selected.ignored"
        class="mt-3"
      />

      <p
        v-if="selected.status.kind === 'bad_signature'"
        class="text-xs text-danger mt-2 flex gap-1"
      >
        <BaseIcon :icon="TriangleAlert" :size="14" class="shrink-0 mt-px" />
        <span
          >This commit's signature does not validate — the commit object may
          have been altered after signing. It cannot be ignored in Enforce
          mode.</span
        >
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
    </BaseModalShell>
  </main>
</template>
