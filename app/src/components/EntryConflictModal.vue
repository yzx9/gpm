<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<!-- Per-entry edit/delete conflict modal (R026). Two-step like DivergenceModal:
     a selection sheet (pick keep-theirs / keep-mine), then a centered contextual
     confirm whose copy names the entry. The danger action is KEEP-THEIRS (it
     discards YOUR work), matching DivergenceModal's "discard local" convention;
     keep-mine is the informed-overwrite secondary. An opt-in "Preview their
     version" button reveals the teammate's current value under the same
     secure-reveal contract as Show Password (FLAG_SECURE + auto-clear). -->

<script setup lang="ts">
import {
  showPassword as showPasswordCmd,
  type AppError,
  type EntryConflictChoice,
} from "@/api";
import type { EntryConflictPayload } from "@/composables";
import { Z, useSecretReveal } from "@/composables";
import { useLockState } from "@/composables/useLockState";
import { computed, nextTick, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import BaseButton from "./base/BaseButton.vue";
import BaseModalShell from "./base/BaseModalShell.vue";
import BaseSpinner from "./base/BaseSpinner.vue";

const { t } = useI18n();
const { runWithAuth } = useLockState();
const { password, notes, revealed, clearsInSecs, reveal, clear, withClaim } =
  useSecretReveal();

const props = withDefaults(
  defineProps<{
    /** Non-null shows the selection sheet. The parent nulls it to close. */
    conflict: EntryConflictPayload | null;
    /** A resolve is in flight — spinner on the confirm button. */
    resolving?: boolean;
    /** Resolve error (e.g. the generic "resolve failed" line). */
    error?: string;
  }>(),
  { resolving: false, error: "" },
);

const emit = defineEmits<{
  (e: "resolve", choice: EntryConflictChoice): void;
  (e: "close"): void;
}>();

/** Which action's confirm step is open (null = the selection sheet). */
const pendingChoice = ref<EntryConflictChoice | null>(null);
const headingEl = ref<HTMLHeadingElement | null>(null);
const previewLoading = ref(false);
const previewError = ref("");

const isEdit = computed(() => props.conflict?.op === "edit");
const heading = computed(() =>
  isEdit.value
    ? t("common.entryConflict.headingEdit")
    : t("common.entryConflict.headingDelete"),
);
/** The discard-your-work action — keep-theirs for edit, keep-theirs for delete. */
const keepTheirsLabel = computed(() =>
  isEdit.value
    ? t("common.entryConflict.useTheirsEdit")
    : t("common.entryConflict.keepTheirsDelete"),
);
/** The informed-overwrite/remove action — keep-mine. */
const keepMineLabel = computed(() =>
  isEdit.value
    ? t("common.entryConflict.useMineEdit")
    : t("common.entryConflict.deleteAnyway"),
);

function openConfirm(choice: EntryConflictChoice) {
  pendingChoice.value = choice;
}
function cancelConfirm() {
  pendingChoice.value = null;
}
function confirm() {
  if (!pendingChoice.value) return;
  // Emit and KEEP pendingChoice up so the confirm stays visible with its spinner
  // while the parent runs the resolve (mirrors DivergenceModal).
  emit("resolve", pendingChoice.value);
}
function cancelAll() {
  emit("close");
}

/** Opt-in: reveal the teammate's current value (local HEAD already IS their
 *  version at the conflict moment) under the secure-reveal contract — same
 *  FLAG_SECURE + auto-clear as Show Password. Surfaces a clear error if it can't
 *  decrypt (recipient set changed); keep-theirs stays a stated leap of faith. */
async function previewTheirs() {
  if (!props.conflict || previewLoading.value) return;
  previewError.value = "";
  previewLoading.value = true;
  clear();
  try {
    const claimed = await withClaim(() =>
      runWithAuth(() => showPasswordCmd(props.conflict!.name)),
    );
    if (!claimed) {
      previewError.value = t("common.entryConflict.previewError");
      return;
    }
    reveal(claimed);
  } catch (e) {
    previewError.value =
      (e as AppError)?.message || t("common.entryConflict.previewError");
  } finally {
    previewLoading.value = false;
  }
}

// Move focus to the heading when a step opens (a11y — alertdialog).
watch(
  () => [props.conflict, pendingChoice.value] as const,
  async () => {
    if (!props.conflict) return;
    await nextTick();
    headingEl.value?.focus();
  },
);

// A resolve error returns the user to the sheet to re-choose.
watch(
  () => props.error,
  (e) => {
    if (e) pendingChoice.value = null;
  },
);

// When the modal closes (conflict → null), wipe any previewed secret.
watch(
  () => props.conflict,
  (c) => {
    if (!c) {
      clear();
      previewError.value = "";
      pendingChoice.value = null;
    }
  },
);
</script>

<template>
  <!-- STEP 1 — selection sheet -->
  <BaseModalShell
    v-if="conflict"
    variant="sheet"
    role="alertdialog"
    :aria-label="t('common.entryConflict.sheetAriaLabel')"
    @close="cancelAll"
  >
    <h2
      ref="headingEl"
      class="text-base font-medium mb-1 text-danger"
      tabindex="-1"
    >
      {{ heading }}
    </h2>
    <p class="text-xs text-muted mb-3">{{ t("common.entryConflict.hint") }}</p>

    <div class="ec-name div-block div-danger mb-3">
      <code :title="conflict.name">{{ conflict.name }}</code>
    </div>

    <!-- Opt-in preview of the teammate's current version -->
    <div v-if="revealed" class="ec-preview div-block div-warn mb-3">
      <div class="div-head text-warning">
        {{ t("common.entryConflict.previewHeading") }}
      </div>
      <div class="ec-secret">
        <code>{{ password }}</code>
      </div>
      <p v-if="notes" class="ec-notes text-muted">{{ notes }}</p>
      <p v-if="clearsInSecs > 0" class="text-xs text-muted ec-clears">
        {{ t("common.entryConflict.autoClearsIn", { secs: clearsInSecs }) }}
      </p>
    </div>
    <p v-else-if="previewError" class="text-xs text-danger mb-2" role="alert">
      {{ previewError }}
    </p>
    <button
      v-else
      class="ec-preview-btn"
      :disabled="previewLoading || resolving"
      :aria-label="t('common.entryConflict.previewAriaLabel')"
      @click="previewTheirs"
    >
      <BaseSpinner v-if="previewLoading" />
      {{ t("common.entryConflict.previewLabel") }}
    </button>

    <p v-if="error" class="text-xs text-danger mb-2" role="alert">
      {{ error }}
    </p>

    <div class="flex flex-col gap-2">
      <!-- keep-theirs discards YOUR work → the danger action (D4). -->
      <button class="btn-danger" @click="openConfirm('keep_theirs')">
        {{ keepTheirsLabel }}
      </button>
      <BaseButton variant="outline" block @click="openConfirm('keep_mine')">
        {{ keepMineLabel }}
      </BaseButton>
      <BaseButton size="sm" :disabled="resolving" @click="cancelAll">
        {{ t("common.button.cancel") }}
      </BaseButton>
    </div>
  </BaseModalShell>

  <!-- STEP 2 — contextual confirm, stacked above the sheet -->
  <BaseModalShell
    v-if="conflict && pendingChoice"
    variant="center"
    :z="Z.overlay"
    role="alertdialog"
    :aria-label="
      pendingChoice === 'keep_theirs'
        ? t('common.entryConflict.confirmAriaLabelKeepTheirs')
        : t('common.entryConflict.confirmAriaLabelKeepMine')
    "
    :dismiss-on-back="!resolving"
    :dismiss-on-backdrop="!resolving"
    @close="cancelConfirm"
  >
    <h2
      ref="headingEl"
      class="text-base font-medium mb-2 text-danger"
      tabindex="-1"
    >
      <template v-if="pendingChoice === 'keep_theirs'">
        {{ t("common.entryConflict.confirmKeepTheirsHeading") }}
      </template>
      <template v-else-if="isEdit">
        {{ t("common.entryConflict.confirmKeepMineEditHeading") }}
      </template>
      <template v-else>
        {{ t("common.entryConflict.confirmDeleteHeading") }}
      </template>
    </h2>

    <p class="text-sm mb-3">
      <template v-if="pendingChoice === 'keep_theirs'">
        {{
          t("common.entryConflict.confirmKeepTheirsLine1", {
            name: conflict!.name,
          })
        }}
      </template>
      <template v-else-if="isEdit">
        {{
          t("common.entryConflict.confirmKeepMineEditLine1", {
            name: conflict!.name,
          })
        }}
      </template>
      <template v-else>
        {{
          t("common.entryConflict.confirmDeleteLine1", { name: conflict!.name })
        }}
      </template>
    </p>

    <div class="flex flex-col gap-2">
      <button class="btn-danger" :disabled="resolving" @click="confirm">
        <BaseSpinner v-if="resolving" />
        <template v-if="resolving">
          {{
            pendingChoice === "keep_theirs"
              ? t("common.entryConflict.discarding")
              : isEdit
                ? t("common.entryConflict.overwriting")
                : t("common.entryConflict.deleting")
          }}
        </template>
        <template v-else>
          {{
            pendingChoice === "keep_theirs"
              ? t("common.entryConflict.discardMyEdit")
              : isEdit
                ? t("common.entryConflict.overwrite")
                : t("common.entryConflict.deleteBtn")
          }}
        </template>
      </button>
      <BaseButton size="sm" :disabled="resolving" @click="cancelConfirm">
        {{ t("common.button.cancel") }}
      </BaseButton>
    </div>
  </BaseModalShell>
</template>

<style scoped>
/* Mirrors DivergenceModal's danger button + divergence block styles so the two
   conflict modals read as one family. */
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

.ec-name code,
.ec-secret code {
  font-size: var(--text-sm);
  word-break: break-all;
}
.ec-notes {
  font-size: var(--text-xs);
  white-space: pre-wrap;
  word-break: break-all;
  margin-top: 0.25rem;
}
.ec-clears {
  margin-top: 0.25rem;
}

.ec-preview-btn {
  background: none;
  border: none;
  color: var(--color-muted);
  font-size: var(--text-xs);
  padding: 0.25rem 0;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  gap: 0.4rem;
  margin-bottom: 0.5rem;
}
.ec-preview-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>
