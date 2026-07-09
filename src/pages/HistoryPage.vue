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
import {
  isSignatureIgnorable,
  signatureSignerFp,
  useToast,
} from "@/composables";
import { formatRelativeTime } from "@/utils/format";
import {
  GitCommitHorizontal,
  History,
  RefreshCw,
  Settings,
  TriangleAlert,
  X,
} from "@lucide/vue";
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";

const { t, locale } = useI18n();
const router = useRouter();
const { toast } = useToast();

const PAGE_SIZE = 50;
const commits = ref<CommitSigInfo[]>([]);
const hasMore = ref(false);
const loading = ref(false);
const error = ref("");
let reqId = 0; // monotonic; bumped per fetch so stale page responses are dropped

const selected = ref<CommitSigInfo | null>(null);
const actionLoading = ref(false);

// ── Infinite-scroll sentinel ────────────────────────────────────────────
const sentinel = ref<HTMLElement | null>(null);
let io: IntersectionObserver | null = null;

const now = ref(Date.now());
let tickTimer: ReturnType<typeof setInterval> | null = null;

const relativeNow = computed(() => now.value);

// Fetch one page from the backend. `replace` swaps page 0 in; otherwise the
// page is appended (load-more). A monotonic request-id guard drops any page
// response that lands after a newer fetch (a trust/ignore reset or a refresh).
// On a replace failure we clear + surface the error; a load-more failure just
// toasts and keeps what's already loaded.
async function fetchPage(offset: number, replace: boolean) {
  const myId = ++reqId;
  loading.value = true;
  try {
    const page = await listCommitSignatures(offset, PAGE_SIZE);
    if (myId !== reqId) return; // superseded by a newer reset/refresh
    commits.value = replace ? page.commits : commits.value.concat(page.commits);
    hasMore.value = page.has_more;
    error.value = "";
  } catch (e) {
    if (myId !== reqId) return;
    const appError = e as AppError;
    if (replace) {
      commits.value = [];
      hasMore.value = false;
      error.value = appError?.message || t("history.loadFailed");
    } else {
      // load-more: keep the already-loaded pages, just surface the error.
      toast.danger(appError?.message || t("history.loadFailed"));
    }
  } finally {
    if (myId === reqId) loading.value = false;
  }
}

function loadMore() {
  if (!hasMore.value || loading.value) return;
  void fetchPage(commits.value.length, false);
}

function openDetail(commit: CommitSigInfo) {
  selected.value = commit;
}

function closeDetail() {
  selected.value = null;
}

async function onTrust(commit: CommitSigInfo) {
  const fp = signatureSignerFp(commit.status);
  const suggested = fp
    ? fp.replace("SHA256:", "").slice(0, 12)
    : t("history.signerDefault");
  const label = window.prompt(t("history.trustPrompt"), suggested);
  if (label === null) return;
  actionLoading.value = true;
  try {
    await trustCommitSigner(commit.hash, label.trim() || suggested);
    toast.success(t("history.trustedToast", { label: label || suggested }));
    // Trust re-derives status for every commit by this signer (possibly outside
    // the loaded pages), so reset to page 0 to refresh the whole view.
    await fetchPage(0, true);
    selected.value = null;
  } catch (e) {
    const appError = e as AppError;
    toast.danger(appError?.message || t("history.trustFailed"));
  } finally {
    actionLoading.value = false;
  }
}

async function onIgnore(commit: CommitSigInfo) {
  actionLoading.value = true;
  try {
    // ignore_commit_issue returns the updated signature info — refresh this row
    // in place (preserving scroll position + other rows) instead of reloading.
    const fresh = await ignoreCommitIssue(commit.hash);
    commits.value = commits.value.map((c) =>
      c.hash === fresh.hash ? fresh : c,
    );
    toast.success(t("history.ignoredToast"));
    selected.value = null;
  } catch (e) {
    const appError = e as AppError;
    toast.danger(appError?.message || t("history.ignoreFailed"));
  } finally {
    actionLoading.value = false;
  }
}

async function copyHash(commit: CommitSigInfo) {
  try {
    await navigator.clipboard.writeText(commit.hash);
    toast.success(t("history.hashCopied"));
  } catch {
    toast.danger(t("common.toast.copyFailed"));
  }
}

function openSettings() {
  // Forward nav, not a pop: History is reached from both Settings and the entry
  // list, so the Settings button must always go to Settings regardless of the
  // opener.
  router.push({ name: "settings" });
}

onMounted(() => {
  void fetchPage(0, true);
  tickTimer = setInterval(() => {
    now.value = Date.now();
  }, 60_000);
  // IntersectionObserver is a progressive enhancement — some WebViews lack it,
  // so the explicit "Load more" button below remains the always-available path.
  if (typeof IntersectionObserver !== "undefined") {
    io = new IntersectionObserver(
      (changes) => {
        if (changes.some((c) => c.isIntersecting)) loadMore();
      },
      { rootMargin: "200px" },
    );
    nextTick(() => {
      if (sentinel.value && io) io.observe(sentinel.value);
    });
  }
});

onBeforeUnmount(() => {
  if (tickTimer) {
    clearInterval(tickTimer);
    tickTimer = null;
  }
  io?.disconnect();
  io = null;
  reqId++; // drop any in-flight page response that lands after unmount
});
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <header class="flex justify-between items-center mb-4" role="banner">
      <h1 class="text-xl flex items-center gap-1">
        <BaseIcon :icon="History" :size="24" /> {{ t("history.title") }}
      </h1>
      <div class="flex gap-2">
        <BaseButton
          size="sm"
          :disabled="loading"
          @click="fetchPage(0, true)"
          :aria-label="t('history.recheckAria')"
          :title="t('history.recheckAria')"
        >
          <BaseIcon :icon="RefreshCw" /> {{ t("history.recheck") }}
        </BaseButton>
        <BaseButton
          size="sm"
          @click="openSettings"
          :aria-label="t('history.settingsAria')"
        >
          <BaseIcon :icon="Settings" />
        </BaseButton>
      </div>
    </header>

    <p class="text-xs text-muted mb-4">{{ t("history.preamble") }}</p>

    <BaseAlert v-if="error" variant="danger" class="mb-3">
      {{ error }}
    </BaseAlert>

    <div
      v-if="loading && commits.length === 0"
      class="flex items-center justify-center gap-2 text-center text-muted py-8"
    >
      <BaseSpinner />
      <span>{{ t("history.loading") }}</span>
    </div>
    <div
      v-else-if="commits.length === 0 && !error"
      class="text-center text-muted py-8"
    >
      <BaseIcon
        :icon="GitCommitHorizontal"
        :size="40"
        class="block mb-2 mx-auto text-muted"
      />
      <p>{{ t("history.empty") }}</p>
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
            commit.subject || t("history.noMessage")
          }}</span>
          <div class="flex items-center gap-1.5 text-xs text-muted min-w-0">
            <code class="shrink-0">{{ commit.short_hash }}</code>
            <span aria-hidden="true" class="shrink-0">·</span>
            <span class="truncate min-w-0">{{ commit.author }}</span>
            <span aria-hidden="true" class="shrink-0">·</span>
            <span class="shrink-0">{{
              formatRelativeTime(relativeNow, Date.parse(commit.date), locale)
            }}</span>
          </div>
        </div>
        <span
          v-if="commit.ignored"
          class="text-[0.6rem] text-default shrink-0 mt-0.5 px-1 rounded-sm bg-edge"
          >{{ t("history.ignoredBadge") }}</span
        >
      </li>
    </ul>

    <div v-if="hasMore" class="flex justify-center py-3">
      <BaseButton
        size="sm"
        :disabled="loading"
        :aria-label="t('history.loadMoreAria')"
        @click="loadMore"
      >
        {{ loading ? t("history.loadMoreLoading") : t("history.loadMore") }}
      </BaseButton>
    </div>
    <div ref="sentinel" class="h-1" aria-hidden="true"></div>

    <!-- Detail sheet -->
    <BaseModalShell
      v-if="selected"
      variant="sheet"
      :aria-label="t('history.detailAria')"
      @close="closeDetail"
    >
      <div class="flex justify-between items-start mb-2">
        <code class="text-xs text-muted">{{ selected.short_hash }}</code>
        <button
          class="btn-copy"
          @click="closeDetail"
          :aria-label="t('history.closeAria')"
        >
          <BaseIcon :icon="X" />
        </button>
      </div>

      <h2 class="text-base font-medium wrap-break-word">
        {{ selected.subject || t("history.noMessage") }}
      </h2>
      <p class="text-xs text-muted mt-1 wrap-break-word">
        {{ selected.author }}
      </p>
      <p class="text-xs text-muted mt-0.5">{{ selected.date }}</p>

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
        <span>{{ t("history.badSigNote") }}</span>
      </p>

      <div class="flex flex-col gap-2 mt-4">
        <p
          v-if="selected.status.kind === 'unverified_signature'"
          class="text-xs text-muted break-words"
        >
          {{ t("history.unverifiedNote") }}
          <span v-if="signatureSignerFp(selected.status)">
            {{ t("history.issuerFp") }}
            <code class="break-all">{{
              signatureSignerFp(selected.status)
            }}</code>
          </span>
        </p>
        <BaseButton
          v-if="selected.status.kind === 'untrusted_key'"
          variant="action"
          :disabled="actionLoading"
          @click="onTrust(selected)"
        >
          {{ t("history.trustSigner") }}
        </BaseButton>
        <BaseButton
          v-if="isSignatureIgnorable(selected.status) && !selected.ignored"
          variant="action"
          :disabled="actionLoading"
          @click="onIgnore(selected)"
        >
          {{ t("history.ignoreIssue") }}
        </BaseButton>
        <BaseButton variant="action" @click="copyHash(selected)">
          {{ t("history.copyHash") }}
        </BaseButton>
      </div>
    </BaseModalShell>
  </main>
</template>
