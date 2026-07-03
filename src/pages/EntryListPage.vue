<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import {
  ref,
  computed,
  watch,
  onMounted,
  onBeforeUnmount,
  nextTick,
} from "vue";
import { useRouter } from "vue-router";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  cancelGit,
  copyPassword as copyPasswordCmd,
  getAuthenticityState,
  ignoreCommitIssue,
  listEntries,
  resolveSyncDivergence,
  searchEntries,
  setVerificationMode,
  subscribeGitProgress,
  syncRepo as syncRepoCmd,
  trustCommitSigner,
  type AppError,
  type AuthenticityState,
  type CommitSigInfo,
  type DivergenceChoice,
  type Entry,
  type GitProgressEvent,
  type SyncDivergence,
} from "@/api";
import { formatRelativeTime } from "@/utils/format";
import { statusLabel } from "@/utils/signature";
import AuthenticityBlockModal from "@/components/AuthenticityBlockModal.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseSpinner from "@/components/base/BaseSpinner.vue";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseModalShell from "@/components/base/BaseModalShell.vue";
import CommitSigIndicator from "@/components/CommitSigIndicator.vue";
import DivergenceModal from "@/components/DivergenceModal.vue";
import { isAuthCancelled, useLockState } from "@/composables";

const router = useRouter();
const { runWithAuth } = useLockState();

// Entries are paginated: the WebView holds only the pages the user has loaded,
// not the whole store. `displayedEntries` accumulates appended pages; `total`
// and `hasMore` drive the "Load more" affordance. `search` decides whether a
// fetch hits the search or list command (empty query == browse).
const PAGE_SIZE = 50;
const displayedEntries = ref<Entry[]>([]);
const total = ref(0);
const hasMore = ref(false);
const activeQuery = ref(""); // the query the currently-displayed pages belong to
const search = ref("");
const searchError = ref(false);
let searchTimer: ReturnType<typeof setTimeout> | null = null;
let reqId = 0; // monotonic; bumped per fetch so stale page responses are dropped
const loading = ref(false);
const pulling = ref(false);
const error = ref("");
const pullResult = ref("");
const pullProgressText = ref("");
const pullProgressPercent = ref(0);
let pullProgressUnlisten: UnlistenFn | null = null;
const toast = ref("");
let toastTimer: ReturnType<typeof setTimeout> | null = null;

const lastSyncTime = ref<number | null>(null);
const now = ref(Date.now());
let tickTimer: ReturnType<typeof setInterval> | null = null;

// ── Infinite-scroll sentinel ────────────────────────────────────────────
const sentinel = ref<HTMLElement | null>(null);
let io: IntersectionObserver | null = null;

// ── Authenticity (badge + pull modals) ───────────────────────────────────
const authState = ref<AuthenticityState | null>(null);
/** Audit-mode open issues from the last pull → drives the mismatch modal. */
const auditIssues = ref<CommitSigInfo[] | null>(null);
/** Enforce-block result from the last pull → drives the block modal. */
const blockIssues = ref<CommitSigInfo[] | null>(null);

// ── Sync divergence (keep-mine / adopt-remote modal) ─────────────────────
/** Diverged sync → drives the shared resolve modal. */
const divergence = ref<SyncDivergence | null>(null);
const resolving = ref(false);
const divergeError = ref("");

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

const remaining = computed(() =>
  Math.max(0, total.value - displayedEntries.value.length),
);

// Fetch one page from the backend. `replace` swaps page 0 in; otherwise the
// page is appended (load-more). A monotonic request-id guard drops any page
// response that lands after a newer fetch (a newer keystroke, a pull, a retry).
// On a search page-0 failure we fall back to browse page 0 + toast (never a
// misleading "No matches"); on a browse page-0 failure we surface the retry
// box; a load-more failure just toasts and keeps what's already loaded.
async function fetchPage(q: string, offset: number, replace: boolean) {
  const myId = ++reqId;
  loading.value = true;
  try {
    const searching = q.trim().length > 0;
    const page = searching
      ? await searchEntries(q, offset, PAGE_SIZE)
      : await listEntries(offset, PAGE_SIZE);
    if (myId !== reqId) return; // superseded by a newer query/reset/pull
    displayedEntries.value = replace
      ? page.entries
      : displayedEntries.value.concat(page.entries);
    total.value = page.total;
    hasMore.value = page.has_more;
    activeQuery.value = q; // load-more continues whatever is displayed
    error.value = "";
    searchError.value = false;
  } catch (e) {
    if (myId !== reqId) return;
    const msg = (e as AppError)?.message || "Failed to load entries";
    if (replace && q.trim()) {
      searchError.value = true;
      showToast(msg);
      void fetchPage("", 0, true); // fall back to browse page 0
    } else if (replace) {
      displayedEntries.value = [];
      total.value = 0;
      hasMore.value = false;
      error.value = msg;
    } else {
      showToast(msg); // load-more: keep the already-loaded pages
    }
  } finally {
    if (myId === reqId) loading.value = false;
  }
}

function loadMore() {
  if (!hasMore.value || loading.value) return;
  void fetchPage(activeQuery.value, displayedEntries.value.length, false);
}

function retry() {
  void fetchPage("", 0, true);
}

// Debounced fuzzy search (150 ms). Clearing the query drops straight back to
// browse page 0; typing re-fetches page 0 of the new query once the user pauses.
watch(search, (q) => {
  if (searchTimer) {
    clearTimeout(searchTimer);
    searchTimer = null;
  }
  if (!q.trim()) {
    void fetchPage("", 0, true);
    return;
  }
  searchTimer = setTimeout(() => void fetchPage(q.trim(), 0, true), 150);
});

async function loadAuthState() {
  try {
    authState.value = await getAuthenticityState();
  } catch {
    // Verification unavailable (e.g. repo mid-clone) — leave the badge as-is.
  }
}

function onPullProgress(p: GitProgressEvent) {
  if (p.total_objects > 0) {
    pullProgressPercent.value = Math.min(
      100,
      Math.round((p.received_objects / p.total_objects) * 100),
    );
  }
  pullProgressText.value =
    p.message ?? `${p.received_objects} / ${p.total_objects} objects`;
}

/** User-initiated cancel of an in-flight sync. */
async function cancelSync() {
  try {
    await cancelGit();
  } catch {
    // best-effort — the sync continues if cancel fails
  }
}

/** The header button starts a sync (pull + push), or cancels one in flight. */
function toggleSync() {
  if (pulling.value) {
    void cancelSync();
  } else {
    void syncRepo();
  }
}

async function syncRepo() {
  pulling.value = true;
  pullResult.value = "";
  error.value = "";
  auditIssues.value = null;
  blockIssues.value = null;
  pullProgressText.value = "Syncing…";
  pullProgressPercent.value = 0;
  pullProgressUnlisten ??= await subscribeGitProgress(onPullProgress);
  try {
    const result = await syncRepoCmd();
    if (result.kind === "diverged") {
      // Surface the divergence for resolution instead of erroring.
      divergence.value = result;
      divergeError.value = "";
      return;
    }
    if (result.changed) {
      pullResult.value = `Updated to ${result.head}`;
      await fetchPage(search.value.trim(), 0, true);
      lastSyncTime.value = Date.now();
    } else {
      pullResult.value = "Already up to date";
    }
    // Refresh the badge with the new HEAD state.
    await loadAuthState();

    // Audit mismatch → informational modal (sync already succeeded).
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
    if (appError?.code === "CANCELLED") {
      // User-initiated abort — neutral status, not an error.
      pullResult.value = "Sync cancelled";
      setTimeout(() => {
        pullResult.value = "";
      }, 3000);
    } else {
      error.value = appError?.message || "Sync failed";
    }
  } finally {
    pullProgressUnlisten?.();
    pullProgressUnlisten = null;
    pulling.value = false;
  }
}

/** Resolve a sync divergence per the user's choice. `keep_mine` re-encrypts
 *  local-only entries onto the reviewed remote tip and pushes — identity-gated
 *  (runWithAuth). `adopt_remote` is a fast-forward (no identity needed). */
async function resolveDivergence(choice: DivergenceChoice) {
  if (!divergence.value) return;
  resolving.value = true;
  divergeError.value = "";
  const expectedRemoteOid = divergence.value.remote_tip;
  try {
    const result =
      choice === "keep_mine"
        ? await runWithAuth(() =>
            resolveSyncDivergence(expectedRemoteOid, choice),
          )
        : await resolveSyncDivergence(expectedRemoteOid, choice);
    divergence.value = null;
    pullResult.value = `Updated to ${result.head}`;
    await fetchPage(search.value.trim(), 0, true);
    lastSyncTime.value = Date.now();
    await loadAuthState();
    // Enforce may refuse the resolve (unverified remote commits) — surface it.
    if (result.authenticity.blocked) {
      blockIssues.value = result.authenticity.open_issues;
    }
    setTimeout(() => {
      pullResult.value = "";
    }, 3000);
  } catch (e) {
    if (isAuthCancelled(e)) return;
    const appError = e as AppError;
    if (appError?.code === "PULL_FF_FAILED") {
      // Remote moved since the user reviewed the divergence — recheck.
      divergeError.value = "";
      divergence.value = null;
      await syncRepo();
    } else {
      divergeError.value = appError?.message || "Resolve failed";
    }
  } finally {
    resolving.value = false;
  }
}

/** Dismiss the divergence modal without changing anything. Sync-context
 *  divergences never deferred an identity wipe, so nothing to discard. */
function cancelDivergence() {
  divergence.value = null;
  divergeError.value = "";
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
    await ignoreCommitIssue(commit.hash);
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
    await trustCommitSigner(commit.hash, label.trim() || commit.short_hash);
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
    await setVerificationMode("audit");
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
    const result = await runWithAuth(() => copyPasswordCmd(entry.path));
    showToast(
      `✓ Copied ${result.entry_name} (${result.cleared_after_secs}s auto-clear)`,
    );
  } catch (e) {
    if (isAuthCancelled(e)) return;
    const appError = e as AppError;
    showToast(`Failed: ${appError?.message || "Copy failed"}`);
  }
}

function openEntry(entry: Entry) {
  router.push({ name: "entry", params: { pathMatch: entry.path } });
}

function openCreate() {
  router.push({ name: "create" });
}

function openGenerate() {
  router.push({ name: "generate" });
}

function openSettings() {
  router.push({ name: "settings" });
}

function openHistory() {
  router.push({ name: "history" });
}

onMounted(() => {
  void fetchPage("", 0, true); // initial browse page 0
  loadAuthState();
  tickTimer = setInterval(() => {
    now.value = Date.now();
  }, 60_000);
  // Infinite scroll: when the sentinel nears the viewport, load the next page.
  // Feature-detected — some WebViews lack IntersectionObserver, so the explicit
  // "Load more" button remains the always-available fallback.
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
  pullProgressUnlisten?.();
  pullProgressUnlisten = null;
  io?.disconnect();
  io = null;
  if (tickTimer) {
    clearInterval(tickTimer);
    tickTimer = null;
  }
  if (searchTimer) {
    clearTimeout(searchTimer);
    searchTimer = null;
  }
  reqId++; // drop any in-flight page response landing after unmount
});
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <header class="flex justify-between items-center mb-4" role="banner">
      <h1 class="text-xl">🔐 gpm</h1>
      <div class="flex gap-2 items-center">
        <BaseButton
          size="sm"
          aria-label="Create a new secret"
          title="Create a new secret"
          @click="openCreate"
        >
          <span aria-hidden="true">＋</span>
        </BaseButton>
        <button
          @click="openHistory"
          class="badge-btn"
          :class="badge.cls"
          :aria-label="badge.title"
          :title="badge.title"
        >
          <span aria-hidden="true">{{ badge.glyph }}</span>
        </button>
        <BaseButton
          size="sm"
          :aria-label="pulling ? 'Cancel sync' : 'Sync with remote'"
          :title="pulling ? 'Cancel sync' : 'Sync with remote'"
          @click="toggleSync"
        >
          <span aria-hidden="true">{{ pulling ? "✕" : "↻" }}</span>
          {{ pulling ? "Cancel" : "Sync" }}
        </BaseButton>
        <BaseButton
          size="sm"
          aria-label="Generate passwords"
          title="Generate passwords"
          @click="openGenerate"
        >
          <span aria-hidden="true">🎲</span>
        </BaseButton>
        <BaseButton
          size="sm"
          aria-label="Settings"
          title="Settings"
          @click="openSettings"
        >
          <span aria-hidden="true">⚙</span>
        </BaseButton>
      </div>
    </header>

    <div v-if="pulling" class="pull-progress">
      <div
        class="pull-progress-track"
        role="progressbar"
        :aria-valuenow="pullProgressPercent"
        aria-valuemin="0"
        aria-valuemax="100"
      >
        <div
          class="pull-progress-fill"
          :style="{ width: `${pullProgressPercent}%` }"
        ></div>
      </div>
      <div class="text-xs text-muted mt-1" aria-live="polite">
        {{ pullProgressText }}
      </div>
    </div>

    <div
      v-if="lastSyncLabel"
      class="text-xs text-subtle text-center mb-2"
      aria-live="polite"
      role="status"
    >
      Last synced {{ lastSyncLabel }}
    </div>

    <div class="mb-4">
      <BaseInput
        v-model="search"
        type="search"
        placeholder="Search entries..."
        class="w-full"
      />
    </div>

    <BaseAlert
      v-if="error"
      variant="danger"
      class="flex justify-between items-center mb-3"
    >
      {{ error }}
      <button @click="retry" class="btn-retry">Retry</button>
    </BaseAlert>
    <BaseAlert v-if="pullResult" variant="info" class="mb-3">
      {{ pullResult }}
    </BaseAlert>
    <BaseAlert v-if="toast" variant="success" class="mb-3">
      {{ toast }}
    </BaseAlert>

    <div
      v-if="loading && displayedEntries.length === 0"
      class="flex items-center justify-center gap-2 text-center text-muted py-8"
    >
      <BaseSpinner />
      <span>Loading entries...</span>
    </div>
    <div
      v-else-if="displayedEntries.length === 0 && !error"
      class="text-center text-muted py-8"
    >
      <template v-if="search.trim() && !searchError">
        <span class="text-4xl block mb-2">🔍</span>
        <p>No matches for "{{ search }}"</p>
      </template>
      <template v-else>
        <span class="text-4xl block mb-2">🔒</span>
        <p>No passwords yet</p>
        <p class="text-xs text-subtle mt-1">Sync, or check your repository</p>
      </template>
    </div>

    <ul v-else class="list-none flex flex-col gap-0.5" role="list">
      <li
        v-for="entry in displayedEntries"
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

    <!-- Load more (explicit infinite-scroll fallback) -->
    <div v-if="hasMore" class="flex justify-center py-3">
      <BaseButton
        size="sm"
        :loading="loading"
        :disabled="loading"
        aria-label="Load more entries"
        @click="loadMore"
      >
        {{ loading ? "Loading…" : `Load more (${remaining} more)` }}
      </BaseButton>
    </div>
    <!-- Sentinel the IntersectionObserver watches to auto-load the next page -->
    <div ref="sentinel" class="h-1" aria-hidden="true"></div>

    <!-- Audit-mode mismatch modal (pull succeeded; informational) -->
    <BaseModalShell
      v-if="auditIssues"
      variant="sheet"
      aria-label="Signature check"
    >
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
          <CommitSigIndicator :status="c.status" />
          <code class="text-xs text-muted">{{ c.short_hash }}</code>
          <span class="flex-1 truncate">{{ c.subject }}</span>
          <span class="text-xs text-muted">{{ statusLabel(c.status) }}</span>
        </li>
      </ul>
      <div class="flex gap-2">
        <BaseButton size="sm" class="flex-1" @click="openHistory">
          Review in history
        </BaseButton>
        <BaseButton
          v-if="auditIssues.length === 1"
          size="sm"
          class="flex-1"
          @click="ignoreIssue(auditIssues[0]!)"
        >
          Ignore this commit
        </BaseButton>
        <BaseButton size="sm" class="flex-1" @click="auditIssues = null">
          Dismiss
        </BaseButton>
      </div>
    </BaseModalShell>

    <<<<<<< HEAD
    <!-- Enforce-block modal (HEAD did not advance) -->
    <BaseModalShell
      v-if="blockIssues"
      variant="sheet"
      aria-label="Pull blocked"
    >
      <h2 class="text-base font-medium mb-1 text-danger">Pull blocked</h2>
      <p class="text-xs text-muted mb-3">
        Enforce mode refused to update the store — HEAD did not advance. Resolve
        the signature issue, then pull again.
      </p>
      <ul class="flex flex-col gap-2 mb-3">
        <li
          v-for="c in blockIssues"
          :key="c.hash"
          class="flex items-center gap-2 text-sm"
        >
          <CommitSigIndicator :status="c.status" />
          <code class="text-xs text-muted">{{ c.short_hash }}</code>
          <span class="flex-1 truncate">{{ c.subject }}</span>
          <span class="text-xs text-muted">{{ statusLabel(c.status) }}</span>
        </li>
      </ul>
      <div class="flex flex-col gap-2">
        <BaseButton
          v-if="blockIssues.some((c) => c.status.kind === 'untrusted_key')"
          size="sm"
          @click="
            trustBlockSigner(
              blockIssues.find((c) => c.status.kind === 'untrusted_key')!,
            )
          "
        >
          Trust this signer
        </BaseButton>
        <BaseButton size="sm" @click="switchToAudit">
          Switch to Audit mode
        </BaseButton>
        <BaseButton size="sm" @click="blockIssues = null">Cancel</BaseButton>
      </div>
    </BaseModalShell>
    <!-- Divergence modal (local & remote diverged) -->
    <BaseModalShell
      v-if="divergence"
      variant="sheet"
      role="alertdialog"
      aria-label="Local and remote have diverged"
    >
      <h2 class="text-base font-medium mb-1 text-danger">
        Local and remote have diverged
      </h2>
      <p class="text-xs text-muted mb-3">
        Your branch is {{ divergence.local_ahead }}
        {{ divergence.local_ahead === 1 ? "commit" : "commits" }} ahead.
        Adopting the remote discards the local-only changes below — this cannot
        be undone.
      </p>

      <div class="flex flex-col gap-2 mb-3 div-scroll">
        <div
          v-if="divergence.local_only_entries.length"
          class="div-block div-danger"
        >
          <div class="div-head text-danger">
            Will be deleted · {{ divergence.local_only_entries.length }}
          </div>
          <ul class="div-list">
            <li v-for="n in divergence.local_only_entries" :key="n">
              <code>{{ n }}</code>
            </li>
          </ul>
        </div>
        <div
          v-if="divergence.modified_entries.length"
          class="div-block div-warn"
        >
          <div class="div-head text-warning">
            Will be overwritten · {{ divergence.modified_entries.length }}
          </div>
          <ul class="div-list">
            <li v-for="n in divergence.modified_entries" :key="n">
              <code>{{ n }}</code>
            </li>
          </ul>
        </div>
        <div
          v-if="divergence.other_changed_files.length"
          class="div-block div-muted"
        >
          <div class="div-head text-muted">
            Other local changes · {{ divergence.other_changed_files.length }}
          </div>
          <ul class="div-list">
            <li v-for="n in divergence.other_changed_files" :key="n">
              <code>{{ n }}</code>
            </li>
          </ul>
        </div>
      </div>

      <p v-if="adoptError" class="text-xs text-danger mb-2" role="alert">
        {{ adoptError }}
      </p>

      <label class="flex items-start gap-2 text-sm mb-3 cursor-pointer">
        <input
          type="checkbox"
          class="mt-1"
          :disabled="adopting"
          v-model="adoptConfirmed"
        />
        <span>
          I understand this discards my {{ divergence.local_ahead }}
          {{ divergence.local_ahead === 1 ? "commit" : "commits" }} and the
          changes listed above.
        </span>
      </label>

      <div class="flex flex-col gap-2">
        <button
          class="btn-danger"
          :disabled="!adoptConfirmed || adopting"
          @click="adoptRemote"
        >
          <BaseSpinner v-if="adopting" />
          {{ adopting ? "Adopting…" : "Adopt remote (discard local)" }}
        </button>
        <BaseButton size="sm" :disabled="adopting" @click="cancelDivergence">
          Cancel
        </BaseButton>
      </div>
    </BaseModalShell>
    =======
    <!-- Enforce-block modal (shared component; HEAD did not advance) -->
    <AuthenticityBlockModal
      :issues="blockIssues"
      @trust-signer="trustBlockSigner"
      @switch-to-audit="switchToAudit"
      @close="blockIssues = null"
    />
    <!-- Divergence modal (shared component, sync context) -->
    <DivergenceModal
      context="sync"
      :divergence="divergence"
      :resolving="resolving"
      :error="divergeError"
      @resolve="resolveDivergence"
      @close="cancelDivergence"
    />
    >>>>>>> 553584e (feat: divergence modal on save, AutoSync toggle, and manual
    Sync)
  </main>
</template>

<style scoped>
.pull-progress {
  margin-bottom: 0.75rem;
}
.pull-progress-track {
  height: 4px;
  background: var(--color-edge);
  border-radius: 9999px;
  overflow: hidden;
}
.pull-progress-fill {
  height: 100%;
  background: var(--color-accent);
  border-radius: 9999px;
  transition: width 0.2s ease;
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

.btn-danger {
  padding: 0.5rem 0.75rem;
  font-size: var(--text-sm);
  border: 1px solid var(--color-danger);
  color: var(--color-danger);
  border-radius: var(--radius-sm);
  background: var(--color-surface);
  cursor: pointer;
  min-height: 48px;
}

.btn-danger:hover:not(:disabled) {
  background: var(--color-danger);
  color: var(--color-surface);
}

.btn-danger:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.div-scroll {
  max-height: 40vh;
  overflow-y: auto;
}

.div-block {
  border-left: 3px solid var(--color-edge);
  padding-left: 0.5rem;
}

.div-danger {
  border-left-color: var(--color-danger);
}

.div-warn {
  border-left-color: var(--color-warning, #c93);
}

.div-muted {
  border-left-color: var(--color-subtle, #999);
}

.div-head {
  font-size: var(--text-xs);
  font-weight: 500;
  margin-bottom: 0.15rem;
}

.div-list {
  margin: 0;
  padding-left: 1rem;
}

.div-list li {
  font-size: var(--text-xs);
}
</style>
