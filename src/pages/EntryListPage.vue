<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import type { UnlistenFn } from "@/api";
import {
  cancelGit,
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
import AuthenticityBlockModal from "@/components/AuthenticityBlockModal.vue";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import BaseModalShell from "@/components/base/BaseModalShell.vue";
import BaseSpinner from "@/components/base/BaseSpinner.vue";
import CommitSigIndicator from "@/components/CommitSigIndicator.vue";
import DivergenceModal from "@/components/DivergenceModal.vue";
import {
  isAuthCancelled,
  useAppLockState,
  useCommitSignature,
  useLockState,
  usePullToRefresh,
  useToast,
} from "@/composables";
import { formatRelativeTime } from "@/utils/format";
import type { LucideIcon } from "@lucide/vue";
import {
  ChevronRight,
  Circle,
  CircleAlert,
  CircleCheck,
  CircleDashed,
  Lock,
  LockKeyhole,
  Plus,
  RefreshCw,
  Search,
  Settings,
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
import { useRouter } from "vue-router";

const router = useRouter();
const { runWithAuth, overlayUp } = useLockState();
const { appLocked } = useAppLockState();
const { toast } = useToast();
const { t, locale } = useI18n();
const { signatureLabel } = useCommitSignature();

// Entries are paginated: the WebView holds only the pages the user has loaded,
// not the whole store. `displayedEntries` accumulates appended pages; `total`
// and `hasMore` drive the "Load more" affordance. `search` decides whether a
// fetch hits the search or list command (empty query == browse).
const PAGE_SIZE = 50;
const displayedEntries = ref<Entry[]>([]);
const total = ref(0);
const hasMore = ref(false);
// True once the first page has been applied — "the list state is known", even if
// that state is empty. Distinguishes a never-loaded list (cold start, or the
// sealed fetch still in flight) from a list that loaded as empty, so the
// app-lock recovery can reload the former and leave the latter intact.
const hasFetchedOnce = ref(false);
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
// This divergence handling is sync-context and stays inline (it does not share
// useDivergence()): a sync detects divergence on pull — there is no local
// commit and no deferred identity wipe, so cancel is a no-op; resolve re-runs
// syncRepo on PULL_FF_FAILED; and enforce may refuse again after resolve. The
// save-context pages (create/edit/delete) share useDivergence() instead. If
// these sync-vs-save differences ever collapse, fold this into useDivergence.
/** Diverged sync → drives the shared resolve modal. */
const divergence = ref<SyncDivergence | null>(null);
const resolving = ref(false);
const divergeError = ref("");

/** The indicator badge for the current authenticity state. */
const badge = computed<{ icon: LucideIcon; cls: string; title: string }>(() => {
  const s = authState.value;
  if (!s || s.mode === "off") {
    return {
      icon: Circle,
      cls: "badge-off",
      title: t("entries.badgeOff"),
    };
  }
  switch (s.head_status.kind) {
    case "verified":
      return {
        icon: CircleCheck,
        cls: "badge-ok",
        title: t("entries.badgeTrustedHead"),
      };
    case "unknown":
      return {
        icon: CircleDashed,
        cls: "badge-none",
        title: t("entries.badgeUnchecked"),
      };
    default:
      return {
        icon: CircleAlert,
        cls: "badge-warn",
        title: t("entries.badgeReviewHead", {
          status: signatureLabel(s.head_status),
        }),
      };
  }
});

const lastSyncLabel = computed(() => {
  if (!lastSyncTime.value) return null;
  return formatRelativeTime(now.value, lastSyncTime.value, locale.value);
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
    hasFetchedOnce.value = true; // state is known (even if this page was empty)
  } catch (e) {
    if (myId !== reqId) return;
    const msg = (e as AppError)?.message || t("entries.loadFailed");
    if (replace && q.trim()) {
      searchError.value = true;
      toast.danger(msg);
      void fetchPage("", 0, true); // fall back to browse page 0
    } else if (replace) {
      displayedEntries.value = [];
      total.value = 0;
      hasMore.value = false;
      error.value = msg;
    } else {
      toast.danger(msg); // load-more: keep the already-loaded pages
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

// App-Lock recovery: with the seal gate on, the cold-start list fetch fails
// `SealKeyUnavailable` while `repo.json` is sealed, and the page intentionally
// surfaces that as a "locked" error — it tells the user the content needs an
// unlock, and because the fetch failed no entry data was loaded, so only the
// chrome + message are visible behind the (semi-transparent) AppLockOverlay.
// Abandoning unlock leaves the message in place as a reminder. Once `appLocked`
// flips to false the master key is back in memory, so clear the now-stale error
// and load the list.
//
// The reload fires whenever the list state is NOT already known (`hasFetchedOnce`
// false, or a stale error showing) — not only when the cold-start fetch failed.
// The biometric AppLockOverlay auto-prompts on mount and can resolve before the
// sealed-state fetch's rejection lands (a fast face unlock or a re-prompt), so
// at the unlock edge the fetch may still be in flight: no error, no entries,
// `hasFetchedOnce` still false. Reload then and let the monotonic reqId drop the
// stale in-flight result. A resume re-lock over a list that has already loaded —
// entries OR a genuinely-empty store — is left intact: nothing changed while
// backgrounded, so no refetch (and no empty → spinner → empty flicker).
watch(appLocked, (locked, prev) => {
  if (prev && !locked && (!hasFetchedOnce.value || error.value)) {
    error.value = "";
    void fetchPage(search.value.trim(), 0, true);
  }
});

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
    p.message ??
    t("entries.objectsProgress", {
      received: p.received_objects,
      total: p.total_objects,
    });
}

/** User-initiated cancel of an in-flight sync. */
async function cancelSync() {
  try {
    await cancelGit();
  } catch {
    // best-effort — the sync continues if cancel fails
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
      pullResult.value = t("entries.toastUpdatedTo", { head: result.head });
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
      error.value = appError?.message || t("entries.syncFailed");
    }
  } finally {
    pullProgressUnlisten?.();
    pullProgressUnlisten = null;
    pulling.value = false;
  }
}

// ── Pull-to-refresh ──────────────────────────────────────────────────────
// The gesture state machine lives in the composable (unit-tested there). The
// `enabled` gate suppresses a pull while any modal or the unlock overlay is up,
// so a stray drag can't race an open divergence resolve — that would overwrite
// the `remote_tip` the user is mid-decision on (`resolveDivergence` captures it
// at call time). syncRepo itself needs no identity (pull/push of existing
// commits), so the locked-overlay case is benign, but we still suppress it so a
// pull can't park a resolve on auth mid-flow. The `!pulling.value` term is the
// single-flight guard: a fast double-pull would otherwise re-enter syncRepo,
// racing two sync_repo IPC calls and overwriting/leaking pullProgressUnlisten.
const { pullDistance, armed } = usePullToRefresh({
  onRefresh: () => void syncRepo(),
  enabled: () =>
    !pulling.value &&
    !divergence.value &&
    !blockIssues.value &&
    !auditIssues.value &&
    !overlayUp.value,
});

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
      divergeError.value = appError?.message || t("entries.resolveFailed");
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

async function ignoreIssue(commit: CommitSigInfo) {
  try {
    await ignoreCommitIssue(commit.hash);
    toast.success(t("entries.toastIgnored"));
    // Remove it from the modal list.
    if (auditIssues.value) {
      auditIssues.value = auditIssues.value.filter(
        (c) => c.hash !== commit.hash,
      );
      if (auditIssues.value.length === 0) auditIssues.value = null;
    }
  } catch (e) {
    const appError = e as AppError;
    toast.danger(appError?.message || t("entries.toastIgnoreFailed"));
  }
}

async function trustBlockSigner(commit: CommitSigInfo) {
  const label = window.prompt(
    t("entries.trustSignerPrompt"),
    commit.short_hash,
  );
  if (label === null) return;
  try {
    await trustCommitSigner(commit.hash, label.trim() || commit.short_hash);
    toast.success(t("entries.toastTrusted"));
    blockIssues.value = null;
    await loadAuthState();
  } catch (e) {
    const appError = e as AppError;
    toast.danger(appError?.message || t("entries.toastTrustFailed"));
  }
}

async function switchToAudit() {
  try {
    await setVerificationMode("audit");
    toast.info(t("entries.toastSwitchedAudit"));
    blockIssues.value = null;
    await loadAuthState();
  } catch (e) {
    const appError = e as AppError;
    toast.danger(appError?.message || t("entries.toastSwitchFailed"));
  }
}

function openEntry(entry: Entry) {
  router.push({ name: "entry", params: { pathMatch: entry.path } });
}

function openCreate() {
  router.push({ name: "create" });
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

// Exposed for the test harness: the Sync header button is gone (replaced by
// pull-to-refresh), so the sync-outcome tests drive this directly.
defineExpose({ syncRepo });
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <header class="flex justify-between items-center mb-4" role="banner">
      <div class="flex items-center gap-1">
        <h1 class="text-xl flex items-center gap-1">
          <BaseIcon :icon="LockKeyhole" :size="24" /> gpm
        </h1>
        <button
          @click="openHistory"
          class="sig-light"
          :class="badge.cls"
          :aria-label="badge.title"
          :title="badge.title"
        >
          <BaseIcon :icon="badge.icon" :size="16" />
        </button>
      </div>
      <div class="flex gap-2 items-center">
        <BaseButton
          size="sm"
          :aria-label="t('entries.createSecret')"
          :title="t('entries.createSecret')"
          @click="openCreate"
        >
          <BaseIcon :icon="Plus" />
        </BaseButton>
        <BaseButton
          size="sm"
          :aria-label="t('entries.settings')"
          :title="t('entries.settings')"
          @click="openSettings"
        >
          <BaseIcon :icon="Settings" />
        </BaseButton>
      </div>
    </header>

    <div
      v-if="!pulling"
      class="ptr-indicator"
      aria-hidden="true"
      :style="{ height: `${pullDistance}px` }"
    >
      <span class="ptr-icon-wrap" :class="{ 'ptr-armed': armed }">
        <BaseIcon :icon="RefreshCw" :size="22" />
      </span>
    </div>

    <div v-if="pulling" class="pull-progress">
      <div class="pull-progress-row">
        <span class="pull-spinner" aria-hidden="true">
          <BaseIcon :icon="RefreshCw" :size="18" />
        </span>
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
        <button
          class="cancel-sync"
          :aria-label="t('entries.cancelSync')"
          :title="t('entries.cancelSync')"
          @click="cancelSync"
        >
          <BaseIcon :icon="X" :size="14" />
        </button>
      </div>
      <div class="text-xs text-muted mt-1 text-center" aria-live="polite">
        {{ pullProgressText }}
      </div>
    </div>

    <div
      v-if="lastSyncLabel"
      class="text-xs text-muted text-center mb-2"
      aria-live="polite"
      role="status"
    >
      {{ t("entries.lastSynced", { time: lastSyncLabel }) }}
    </div>

    <div class="mb-4">
      <BaseInput
        v-model="search"
        type="search"
        :placeholder="t('entries.searchPlaceholder')"
        class="w-full"
      />
    </div>

    <BaseAlert
      v-if="error"
      variant="danger"
      class="flex justify-between items-center mb-3"
    >
      {{ error }}
      <button @click="retry" class="btn-retry">{{ t("entries.retry") }}</button>
    </BaseAlert>
    <BaseAlert v-if="pullResult" variant="info" class="mb-3">
      {{ pullResult }}
    </BaseAlert>

    <div
      v-if="loading && displayedEntries.length === 0"
      class="flex items-center justify-center gap-2 text-center text-muted py-8"
    >
      <BaseSpinner />
      <span>{{ t("entries.loadingEntries") }}</span>
    </div>
    <div
      v-else-if="displayedEntries.length === 0 && !error"
      class="text-center text-muted py-8"
    >
      <template v-if="search.trim() && !searchError">
        <BaseIcon
          :icon="Search"
          :size="40"
          class="block mb-2 mx-auto text-muted"
        />
        <p>{{ t("entries.noMatches", { query: search }) }}</p>
      </template>
      <template v-else>
        <BaseIcon
          :icon="Lock"
          :size="40"
          class="block mb-2 mx-auto text-muted"
        />
        <p>{{ t("entries.empty") }}</p>
        <p class="text-xs text-muted mt-1">
          {{ t("entries.emptyHint") }}
        </p>
      </template>
    </div>

    <ul v-else class="list-none flex flex-col gap-0.5" role="list">
      <li v-for="entry in displayedEntries" :key="entry.path">
        <div
          class="flex items-center gap-2 p-[0.6rem_0.75rem] md:p-[0.8rem_1rem] bg-surface rounded-md transition-colors duration-150 min-h-12 hover:bg-hover cursor-pointer active:bg-hover"
          tabindex="0"
          role="button"
          :aria-label="t('entries.openEntry', { name: entry.name })"
          @click="openEntry(entry)"
          @keydown.enter="openEntry(entry)"
          @keydown.space.prevent="openEntry(entry)"
        >
          <div class="flex-1 min-w-0">
            <span
              class="block font-medium whitespace-nowrap overflow-hidden text-ellipsis"
              >{{ entry.name }}</span
            >
          </div>
          <BaseIcon
            :icon="ChevronRight"
            :size="20"
            class="text-muted shrink-0"
          />
        </div>
      </li>
    </ul>

    <!-- Load more (explicit infinite-scroll fallback) -->
    <div v-if="hasMore" class="flex justify-center py-3">
      <BaseButton
        size="sm"
        :loading="loading"
        :disabled="loading"
        :aria-label="t('entries.loadMoreAria')"
        @click="loadMore"
      >
        {{
          loading
            ? t("entries.loadMoreLoading")
            : t("entries.loadMore", { count: remaining })
        }}
      </BaseButton>
    </div>
    <!-- Sentinel the IntersectionObserver watches to auto-load the next page -->
    <div ref="sentinel" class="h-1" aria-hidden="true"></div>

    <!-- Audit-mode mismatch modal (pull succeeded; informational) -->
    <BaseModalShell
      v-if="auditIssues"
      variant="sheet"
      :aria-label="t('entries.auditTitle')"
      :dismiss-on-backdrop="false"
      @close="auditIssues = null"
    >
      <h2 class="text-base font-medium mb-1">{{ t("entries.auditTitle") }}</h2>
      <p class="text-xs text-muted mb-3">
        {{ t("entries.auditPreamble", { count: auditIssues.length }) }}
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
          <span class="text-xs text-muted">{{ signatureLabel(c.status) }}</span>
        </li>
      </ul>
      <div class="flex gap-2">
        <BaseButton size="sm" class="flex-1" @click="openHistory">
          {{ t("entries.auditReview") }}
        </BaseButton>
        <BaseButton
          v-if="auditIssues.length === 1"
          size="sm"
          class="flex-1"
          @click="ignoreIssue(auditIssues[0]!)"
        >
          {{ t("entries.auditIgnore") }}
        </BaseButton>
        <BaseButton size="sm" class="flex-1" @click="auditIssues = null">
          {{ t("entries.auditDismiss") }}
        </BaseButton>
      </div>
    </BaseModalShell>

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
  </main>
</template>

<style scoped>
.pull-progress {
  margin-bottom: 0.75rem;
}
.pull-progress-track {
  flex: 1 1 auto; /* fill the row — restores the full-width bar the pre-PTR sync used */
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

.btn-retry:active {
  opacity: 0.8;
}
@media (hover: hover) {
  .btn-retry:hover {
    opacity: 0.8;
  }
}

.pull-progress-row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}
/* Spinning refresh icon = the primary "syncing" affordance, leading the row so
   the sync reads as active work, not as a lone stop button. Uses the global
   @keyframes spin (see src/style.css). */
.pull-spinner {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: var(--color-accent);
  animation: spin 0.8s linear infinite;
}
/* Secondary: a small, calm stop control trailing the bar — only turns danger-red
   on hover/press, so the in-flight state isn't presented as an alarm. */
.cancel-sync {
  flex: 0 0 auto;
  width: 26px;
  height: 26px;
  min-height: 26px;
  padding: 0;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-sm);
  background: var(--color-surface);
  color: var(--color-muted);
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}
.cancel-sync:active {
  background: var(--color-hover);
  color: var(--color-danger);
}
@media (hover: hover) {
  .cancel-sync:hover {
    background: var(--color-hover);
    color: var(--color-danger);
  }
}

/* Pull-to-refresh indicator: a centered icon whose container grows with the
   pull distance. At rest (0) it collapses out of flow; once the sync starts
   (`pulling`), `v-if="!pulling"` removes it entirely and the progress bar below
   takes over — no stale spinner during the gap between release and sync start. */
.ptr-indicator {
  display: flex;
  align-items: flex-end;
  justify-content: center;
  overflow: hidden;
  color: var(--color-muted);
}
/* Lift the icon off the search box: the indicator's bottom edge is flush with
   the search input, and `align-items: flex-end` glues the icon to that edge, so
   without this padding the spinner sits hard against the input. The padding
   lives on the icon wrap (not the indicator) so it only costs height while a
   pull is in progress — at rest the indicator collapses to 0 and this is moot. */
.ptr-icon-wrap {
  padding-bottom: 0.5rem;
}
.ptr-icon-wrap.ptr-armed {
  color: var(--color-accent);
}

/* Status light next to the logo: visually a small colored icon, but the touch
   target stays ≥48 px (transparent padding around the 16 px icon) so it's an
   accessible tap on Android. Borderless/backgroundless so it reads as a lamp,
   not a toolbar button. */
.sig-light {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 44px;
  min-height: 48px;
  padding: 0;
  margin-left: -0.25rem;
  border: none;
  background: transparent;
  cursor: pointer;
  border-radius: var(--radius-sm);
}
.sig-light:active {
  opacity: 0.7;
}
.sig-light:hover {
  opacity: 0.7;
}
.sig-light:focus-visible {
  outline: 2px solid var(--color-accent);
  outline-offset: 2px;
}
.badge-ok {
  color: var(--color-success);
}
.badge-warn {
  color: var(--color-warning);
}
.badge-off,
.badge-none {
  color: var(--color-muted);
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

.btn-danger:active:not(:disabled) {
  background: var(--color-danger);
  color: var(--color-surface);
}
@media (hover: hover) {
  .btn-danger:hover:not(:disabled) {
    background: var(--color-danger);
    color: var(--color-surface);
  }
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
