<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<script setup lang="ts">
import type { AppError, AttachmentMeta, CommitSigInfo } from "@/api";
import { copyRevision, listRevisions, showRevision } from "@/api";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseHeader from "@/components/base/BaseHeader.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseModalShell from "@/components/base/BaseModalShell.vue";
import BaseSpinner from "@/components/base/BaseSpinner.vue";
import CommitSigIndicator from "@/components/CommitSigIndicator.vue";
import EntryAttributes from "@/components/EntryAttributes.vue";
import {
  isAuthCancelled,
  useLockState,
  useRelativeTime,
  useSecretReveal,
  useToast,
} from "@/composables";
import {
  GitCommitHorizontal,
  History,
  Lock,
  Paperclip,
  RefreshCw,
  Trash2,
  X,
} from "@lucide/vue";
import {
  computed,
  nextTick,
  onBeforeUnmount,
  onMounted,
  ref,
  watch,
} from "vue";
import { useI18n } from "vue-i18n";
import { useRoute } from "vue-router";

const { t } = useI18n();
const { formatRelativeTime } = useRelativeTime();
const { toast } = useToast();
const { runWithAuth } = useLockState();
const {
  attributes,
  password,
  notes,
  revealed,
  clearsInSecs,
  reveal,
  clear,
  withClaim,
} = useSecretReveal();

// The entry this page is the history of: the route's catch-all param is the
// (URL-encoded) `.age` path, decoded like EntryDetailPage.
const route = useRoute();
const pathMatch = route.params.pathMatch;
const entryPath = decodeURIComponent(
  Array.isArray(pathMatch) ? pathMatch[0] : pathMatch,
);
// The secret this is the history of — shown under the title so the page names
// which entry it belongs to (mirrors EntryDetailPage's `entryName`).
const entryName = entryPath.replace(/\.age$/, "");

const PAGE_SIZE = 50;
const commits = ref<CommitSigInfo[]>([]);
const hasMore = ref(false);
const loading = ref(false);
const error = ref("");
let reqId = 0; // monotonic; bumped per fetch so stale page responses are dropped
// the HEAD oid captured on page 0 and passed back on every load-more so a
// background sync can't drift the page window.
let baseOid: string | undefined;
// The newest revision (page 0's first row) is the live value — badge it.
const headHash = ref<string | null>(null);

const selected = ref<CommitSigInfo | null>(null);
// When the shown revision is a binary attachment, its metadata (filename) for
// the attachment notice (no copyable password, no past-version export).
const attachmentMeta = ref<AttachmentMeta | null>(null);
type ViewState =
  | "idle" // sheet shows metadata + Show/Copy affordances
  | "loading"
  | "revealed" // a past value is on screen under the past-version banner
  | "attachment" // the revision is a binary attachment — notice, no reveal
  | "undecryptable"
  | "deleted";
const viewState = ref<ViewState>("idle");
const actionLoading = ref(false);

const now = ref(Date.now());
let tickTimer: ReturnType<typeof setInterval> | null = null;
const relativeNow = computed(() => now.value);

// ── Infinite-scroll sentinel ────────────────────────────────────────────
const sentinel = ref<HTMLElement | null>(null);
let io: IntersectionObserver | null = null;

async function fetchPage(offset: number, replace: boolean) {
  const myId = ++reqId;
  loading.value = true;
  try {
    const page = await listRevisions(
      entryPath,
      offset,
      PAGE_SIZE,
      offset === 0 ? undefined : baseOid,
    );
    if (myId !== reqId) return;
    commits.value = replace ? page.commits : commits.value.concat(page.commits);
    // An empty page is terminal regardless of `has_more`: the scan cap can
    // report "matches may exist" when it found none, and paging further would
    // loop on the same empty window.
    hasMore.value = page.commits.length === 0 ? false : page.has_more;
    if (replace) {
      baseOid = page.base_oid;
      headHash.value = page.commits[0]?.hash ?? null;
    }
    error.value = "";
  } catch (e) {
    if (myId !== reqId) return;
    const appError = e as AppError;
    if (replace) {
      commits.value = [];
      hasMore.value = false;
      error.value = appError?.message || t("revisions.loadFailed");
    } else {
      toast.danger(appError?.message || t("revisions.loadFailed"));
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
  viewState.value = "idle";
  attachmentMeta.value = null;
  clear(); // wipe any prior reveal before reusing the sheet
}

function closeDetail() {
  selected.value = null;
  viewState.value = "idle";
  attachmentMeta.value = null;
  clear();
}

async function showVersion() {
  const commit = selected.value;
  if (!commit) return;
  viewState.value = "loading";
  try {
    const claimed = await withClaim(() =>
      runWithAuth(() => showRevision(entryPath, commit.hash)),
    );
    if (claimed === null) {
      viewState.value = "idle";
      toast.danger(t("common.toast.secureScreenFailed"));
      return;
    }
    if (claimed.kind === "decrypted") {
      if (claimed.attachment) {
        // A past attachment has no copyable password and no viewable body
        // (the base64 wall is withheld). Show the notice instead of revealing.
        attachmentMeta.value = claimed.attachment;
        viewState.value = "attachment";
      } else {
        reveal(claimed); // Claimed<decrypted> structurally satisfies reveal's input
        viewState.value = "revealed";
      }
    } else if (claimed.kind === "undecryptable") {
      viewState.value = "undecryptable";
    } else {
      viewState.value = "deleted";
    }
  } catch (e) {
    if (isAuthCancelled(e)) {
      viewState.value = "idle";
      return;
    }
    viewState.value = "idle";
    toast.danger((e as AppError)?.message || t("revisions.loadFailed"));
  }
}

async function copyVersion() {
  const commit = selected.value;
  if (!commit) return;
  actionLoading.value = true;
  try {
    const result = await runWithAuth(() =>
      copyRevision(entryPath, commit.hash),
    );
    // An attachment revision has no password — the backend skips the clipboard
    // write, so don't claim "Copied" (the clipboard may still hold a prior
    // secret). Mirrors EntryDetailPage.copyPassword's attachment branch.
    if (result.has_attachment) {
      toast.info(t("revisions.copyBlocked"));
      return;
    }
    if (result.password_non_utf8) {
      // The revision's password has non-UTF-8 bytes — the backend skipped the
      // clipboard write. Mirrors EntryDetailPage.copyPassword's non-UTF-8 branch.
      toast.info(t("revisions.nonUtf8CopyBlocked"));
      return;
    }
    toast.success(
      t("revisions.copyToast", {
        date: formatRelativeTime(relativeNow.value, Date.parse(commit.date)),
      }),
    );
  } catch (e) {
    if (!isAuthCancelled(e)) {
      toast.danger((e as AppError)?.message || t("revisions.copyFailed"));
    }
  } finally {
    actionLoading.value = false;
  }
}

async function copyHash(commit: CommitSigInfo) {
  try {
    await navigator.clipboard.writeText(commit.hash);
    toast.success(t("revisions.hashCopied"));
  } catch {
    toast.danger(t("common.toast.copyFailed"));
  }
}

// When the auto-clear timer fires, drop back to the show-button state.
watch(revealed, (r) => {
  if (!r && viewState.value === "revealed") viewState.value = "idle";
});

onMounted(() => {
  void fetchPage(0, true);
  tickTimer = setInterval(() => {
    now.value = Date.now();
  }, 60_000);
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
  reqId++;
  clear();
});
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <BaseHeader
      :back-fallback="{ name: 'entry', params: { pathMatch } }"
      spacing="sm"
      :title="t('revisions.title')"
      :title-icon="History"
    >
      <template #actions>
        <BaseButton
          size="sm"
          :disabled="loading"
          @click="fetchPage(0, true)"
          :aria-label="t('revisions.refreshAria')"
          :title="t('revisions.refreshAria')"
        >
          <BaseIcon :icon="RefreshCw" />
        </BaseButton>
      </template>
    </BaseHeader>

    <p class="text-sm text-default break-all mb-1">{{ entryName }}</p>
    <p class="text-xs text-muted mb-4">{{ t("revisions.preamble") }}</p>

    <BaseAlert v-if="error" variant="danger" class="mb-3">
      {{ error }}
    </BaseAlert>

    <div
      v-if="loading && commits.length === 0"
      class="flex items-center justify-center gap-2 text-center text-muted py-8"
    >
      <BaseSpinner />
      <span>{{ t("revisions.loading") }}</span>
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
      <p>{{ t("revisions.empty") }}</p>
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
            commit.subject || t("revisions.noMessage")
          }}</span>
          <div class="flex items-center gap-1.5 text-xs text-muted min-w-0">
            <code class="shrink-0 select-all">{{ commit.short_hash }}</code>
            <span aria-hidden="true" class="shrink-0">·</span>
            <span class="truncate min-w-0">{{ commit.author }}</span>
            <span aria-hidden="true" class="shrink-0">·</span>
            <span class="shrink-0">{{
              formatRelativeTime(relativeNow, Date.parse(commit.date))
            }}</span>
          </div>
        </div>
        <span
          v-if="commit.hash === headHash"
          class="text-[0.6rem] text-default shrink-0 mt-0.5 px-1 rounded-sm bg-edge"
          >{{ t("revisions.currentBadge") }}</span
        >
        <span
          v-else-if="commit.ignored"
          class="text-[0.6rem] text-default shrink-0 mt-0.5 px-1 rounded-sm bg-edge"
          >{{ t("common.signature.ignored") }}</span
        >
      </li>
    </ul>

    <div v-if="hasMore" class="flex justify-center py-3">
      <BaseButton
        size="sm"
        :disabled="loading"
        :aria-label="t('revisions.loadMoreAria')"
        @click="loadMore"
      >
        {{ loading ? t("revisions.loadMoreLoading") : t("revisions.loadMore") }}
      </BaseButton>
    </div>
    <div ref="sentinel" class="h-1" aria-hidden="true"></div>

    <!-- Detail sheet -->
    <BaseModalShell
      v-if="selected"
      variant="sheet"
      :aria-label="t('revisions.detailAria')"
      @close="closeDetail"
    >
      <div class="flex justify-between items-start mb-2">
        <code class="text-xs text-muted">{{ selected.short_hash }}</code>
        <button
          class="btn-copy"
          @click="closeDetail"
          :aria-label="t('revisions.closeAria')"
        >
          <BaseIcon :icon="X" />
        </button>
      </div>

      <!-- a revealed OLD value is unmistakably marked as a past version. -->
      <BaseAlert
        v-if="viewState === 'revealed'"
        variant="warning"
        class="mb-2 flex items-center gap-2"
      >
        <BaseIcon :icon="History" :size="16" class="shrink-0" />
        <span>
          {{
            t("revisions.pastBanner", {
              date: formatRelativeTime(relativeNow, Date.parse(selected.date)),
              hash: selected.short_hash,
            })
          }}
          <span class="block text-xs">{{ t("revisions.pastBannerSub") }}</span>
        </span>
      </BaseAlert>

      <h2 class="text-base font-medium wrap-break-word">
        {{ selected.subject || t("revisions.noMessage") }}
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

      <!-- Revealed past value: bg-edge so a glance distinguishes it from the
           current-secret reveal in EntryDetailPage. -->
      <div
        v-if="viewState === 'revealed' && revealed"
        class="mt-3 rounded-md bg-edge p-2"
      >
        <div class="font-mono break-all">{{ password }}</div>
        <div
          v-if="notes"
          class="font-mono break-all text-muted mt-1 whitespace-pre-wrap"
        >
          {{ notes }}
        </div>
        <EntryAttributes :attributes="attributes ?? []" class="mt-1" />
        <div v-if="clearsInSecs > 0" class="text-xs text-muted mt-1">
          {{ t("revisions.clearsIn", { secs: clearsInSecs }) }}
        </div>
      </div>

      <BaseAlert
        v-if="viewState === 'attachment'"
        variant="info"
        class="mt-3 flex items-start gap-2"
      >
        <BaseIcon :icon="Paperclip" :size="16" class="shrink-0 mt-0.5" />
        <span>
          <span class="font-medium">{{ t("revisions.attachmentTitle") }}</span>
          <span
            v-if="attachmentMeta?.filename"
            class="block text-xs text-muted font-mono break-all"
            >{{ attachmentMeta.filename }}</span
          >
          <span class="block text-xs">{{ t("revisions.attachmentBody") }}</span>
        </span>
      </BaseAlert>

      <BaseAlert
        v-if="viewState === 'undecryptable'"
        variant="warning"
        class="mt-3 flex items-start gap-2"
      >
        <BaseIcon :icon="Lock" :size="16" class="shrink-0 mt-0.5" />
        <span>
          <span class="font-medium">{{
            t("revisions.undecryptableTitle")
          }}</span>
          <span class="block text-xs">{{
            t("revisions.undecryptableBody")
          }}</span>
        </span>
      </BaseAlert>

      <BaseAlert
        v-if="viewState === 'deleted'"
        variant="info"
        class="mt-3 flex items-start gap-2"
      >
        <BaseIcon :icon="Trash2" :size="16" class="shrink-0 mt-0.5" />
        <span>
          <span class="font-medium">{{ t("revisions.deletedTitle") }}</span>
          <span class="block text-xs">{{ t("revisions.deletedBody") }}</span>
        </span>
      </BaseAlert>

      <div class="flex flex-col gap-2 mt-4">
        <BaseButton
          v-if="viewState === 'idle' || viewState === 'loading'"
          variant="action"
          :disabled="viewState === 'loading'"
          @click="showVersion"
        >
          {{
            viewState === "loading"
              ? t("revisions.loadMoreLoading")
              : t("revisions.showVersion")
          }}
        </BaseButton>
        <BaseButton
          v-if="viewState === 'revealed' && revealed"
          variant="action"
          @click="clear"
        >
          {{ t("revisions.hideVersion") }}
        </BaseButton>
        <BaseButton
          v-if="viewState === 'idle' || viewState === 'revealed'"
          variant="action"
          :disabled="actionLoading"
          @click="copyVersion"
        >
          {{ t("revisions.copyVersion") }}
        </BaseButton>
        <BaseButton variant="action" @click="copyHash(selected)">
          {{ t("revisions.copyHash") }}
        </BaseButton>
      </div>
    </BaseModalShell>
  </main>
</template>
