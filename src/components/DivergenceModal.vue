<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<!-- Shared divergence resolution modal — used by manual Sync (pull/sync path)
     AND by save-triggered divergences (create/edit/delete). Two-step: a
     selection sheet (review the diff, pick Adopt or Keep), then a centered
     contextual confirm whose copy names the exact entries THAT action destroys.
     No confirm-checkbox — the contextual confirm is what gates the destructive
     act, so it carries the right wording whichever button was tapped. -->

<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import type { DivergenceChoice, SyncDivergence } from "@/api";
import BaseButton from "./base/BaseButton.vue";
import BaseModalShell from "./base/BaseModalShell.vue";
import BaseSpinner from "./base/BaseSpinner.vue";

const props = withDefaults(
  defineProps<{
    /** Non-null shows the selection sheet. The parent nulls it to close. */
    divergence: SyncDivergence | null;
    /** A resolve is in flight — shows a spinner on the confirm button. */
    resolving?: boolean;
    /** Wording: a save-triggered divergence vs a manual Sync. */
    context?: "sync" | "save";
    /** Resolve error (e.g. the "remote moved since review" recheck notice). */
    error?: string;
  }>(),
  { resolving: false, context: "sync", error: "" },
);

const emit = defineEmits<{
  (e: "resolve", choice: DivergenceChoice): void;
  (e: "close"): void;
}>();

/** Which action's confirm step is open (null = the selection sheet). */
const pendingChoice = ref<DivergenceChoice | null>(null);
const headingEl = ref<HTMLHeadingElement | null>(null);

const isSave = computed(() => props.context === "save");
const heading = computed(() =>
  isSave.value
    ? "Your edit conflicts with a newer remote"
    : "Local and remote have diverged",
);
const aheadNoun = computed(() =>
  props.divergence && props.divergence.local_ahead === 1 ? "commit" : "commits",
);
/** Everything "adopt remote" discards — the union of all local-only changes. */
const adoptLosses = computed<string[]>(() => {
  const d = props.divergence;
  if (!d) return [];
  return [
    ...d.local_only_entries,
    ...d.modified_entries,
    ...d.other_changed_files,
  ];
});
/** Entries "keep mine" may overwrite on the remote — the shared, modified ones. */
const keepOverwrites = computed(() => props.divergence?.modified_entries ?? []);

function openConfirm(choice: DivergenceChoice) {
  pendingChoice.value = choice;
}
function cancelConfirm() {
  pendingChoice.value = null;
}
function confirm() {
  if (!pendingChoice.value) return;
  // Emit and KEEP pendingChoice up so the confirm stays visible with its spinner
  // while the parent runs the resolve. On success the parent nulls `divergence`
  // (closing both steps); on error the parent sets `error` and the watch below
  // drops back to the selection sheet.
  emit("resolve", pendingChoice.value);
}
function cancelAll() {
  emit("close");
}

// Move focus to the heading when a step opens (a11y — the modal is an
// alertdialog, so focus should land on its title, not stay on the trigger).
watch(
  () => [props.divergence, pendingChoice.value] as const,
  async () => {
    if (!props.divergence) return;
    await nextTick();
    headingEl.value?.focus();
  },
);

// A resolve error returns the user to the selection sheet to re-choose (the
// remote may have moved, changing the diff).
watch(
  () => props.error,
  (e) => {
    if (e) pendingChoice.value = null;
  },
);
</script>

<template>
  <!-- STEP 1 — selection sheet (NO confirm checkbox) -->
  <BaseModalShell
    v-if="divergence"
    variant="sheet"
    role="alertdialog"
    aria-label="Local and remote have diverged"
    @close="cancelAll"
  >
    <h2
      ref="headingEl"
      class="text-base font-medium mb-1 text-danger"
      tabindex="-1"
    >
      {{ heading }}
    </h2>
    <p class="text-xs text-muted mb-3">
      Your branch is {{ divergence.local_ahead }} {{ aheadNoun }} ahead.
      <template v-if="isSave">
        Adopting discards your local edit; keeping it may overwrite the remote
        entries below.
      </template>
      <template v-else>
        Adopting discards the local-only changes below; keeping pushes them (and
        may overwrite the remote entries).
      </template>
    </p>

    <div v-if="adoptLosses.length" class="flex flex-col gap-2 mb-3 div-scroll">
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
      <div v-if="divergence.modified_entries.length" class="div-block div-warn">
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
    <p v-else class="text-xs text-muted mb-3">Nothing differs locally.</p>

    <p v-if="error" class="text-xs text-danger mb-2" role="alert">
      {{ error }}
    </p>

    <div class="flex flex-col gap-2">
      <button class="btn-danger" @click="openConfirm('adopt_remote')">
        Adopt remote (discard local)
      </button>
      <BaseButton variant="outline" block @click="openConfirm('keep_mine')">
        Keep mine (push local)
      </BaseButton>
      <BaseButton size="sm" :disabled="resolving" @click="cancelAll">
        Cancel
      </BaseButton>
    </div>
  </BaseModalShell>

  <!-- STEP 2 — contextual confirm, stacked above the sheet -->
  <BaseModalShell
    v-if="divergence && pendingChoice"
    variant="center"
    :z="70"
    role="alertdialog"
    :aria-label="
      pendingChoice === 'adopt_remote'
        ? 'Discard your local commit'
        : 'Push and overwrite remote'
    "
    @close="cancelConfirm"
  >
    <h2
      ref="headingEl"
      class="text-base font-medium mb-2 text-danger"
      tabindex="-1"
    >
      <template v-if="pendingChoice === 'adopt_remote'">
        Discard your local {{ aheadNoun }}?
      </template>
      <template v-else>Push &amp; overwrite remote?</template>
    </h2>

    <template v-if="pendingChoice === 'adopt_remote'">
      <p class="text-sm mb-2">
        Your {{ divergence!.local_ahead }} local {{ aheadNoun }}
        {{ divergence!.local_ahead === 1 ? "is" : "are" }} lost, including:
      </p>
      <ul v-if="adoptLosses.length" class="div-list mb-3">
        <li v-for="n in adoptLosses" :key="n">
          <code>{{ n }}</code>
        </li>
      </ul>
      <p class="text-xs text-muted mb-3">The remote versions will be kept.</p>
    </template>
    <template v-else>
      <p class="text-sm mb-2">
        Your local changes will be pushed. This may overwrite the remote version
        of:
      </p>
      <ul v-if="keepOverwrites.length" class="div-list mb-3">
        <li v-for="n in keepOverwrites" :key="n">
          <code>{{ n }}</code>
        </li>
      </ul>
      <p v-if="!keepOverwrites.length" class="text-xs text-muted mb-3">
        No shared entries differ — only your local-only entries are pushed.
      </p>
    </template>

    <div class="flex flex-col gap-2">
      <button class="btn-danger" :disabled="resolving" @click="confirm">
        <BaseSpinner v-if="resolving" />
        <template v-if="resolving">
          {{ pendingChoice === "adopt_remote" ? "Discarding…" : "Pushing…" }}
        </template>
        <template v-else>
          {{
            pendingChoice === "adopt_remote"
              ? "Discard my commit"
              : "Push & overwrite"
          }}
        </template>
      </button>
      <BaseButton size="sm" :disabled="resolving" @click="cancelConfirm">
        Cancel
      </BaseButton>
    </div>
  </BaseModalShell>
</template>

<style scoped>
/* Lifted from EntryListPage so the shared modal owns its own list styles and
   doesn't depend on a page's scoped CSS. */
.btn-danger {
  padding: 0.5rem 0.75rem;
  font-size: var(--text-sm);
  border: 1px solid var(--color-danger);
  color: var(--color-danger);
  border-radius: var(--radius-sm);
  background: var(--color-surface);
  cursor: pointer;
  min-height: 48px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 0.4rem;
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
