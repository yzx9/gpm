<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

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
  type EntryConflictOp,
} from "@/api";
import type { EntryConflictPayload } from "@/composables";
import { Z, useSecretReveal } from "@/composables";
import { useLockState } from "@/composables/useLockState";
import { computed, nextTick, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import BaseButton from "./base/BaseButton.vue";
import BaseModalShell from "./base/BaseModalShell.vue";
import EntryAttributes from "./EntryAttributes.vue";

const { t } = useI18n();
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

const op = computed(() => props.conflict?.op ?? "edit");
const heading = computed(
  () =>
    (
      ({
        edit: t("common.entryConflict.headingEdit"),
        delete: t("common.entryConflict.headingDelete"),
        create: t("common.entryConflict.headingCreate"),
      }) as const
    )[op.value],
);
/** The discard-your-work action — keep-theirs (drops your edit/delete/create). */
const keepTheirsLabel = computed(
  () =>
    (
      ({
        edit: t("common.entryConflict.useTheirsEdit"),
        delete: t("common.entryConflict.keepTheirsDelete"),
        create: t("common.entryConflict.keepTheirsCreate"),
      }) as const
    )[op.value],
);
/** The informed-overwrite/remove action — keep-mine. */
const keepMineLabel = computed(
  () =>
    (
      ({
        edit: t("common.entryConflict.useMineEdit"),
        delete: t("common.entryConflict.deleteAnyway"),
        create: t("common.entryConflict.overwriteCreate"),
      }) as const
    )[op.value],
);
// Step-2 contextual confirm copy, keyed by (choice, op). Returns an i18n key.
const confirmHeading = computed(() => {
  const byChoice: Record<
    EntryConflictChoice,
    Record<EntryConflictOp, string>
  > = {
    keep_theirs: {
      edit: "common.entryConflict.confirmKeepTheirsHeading",
      delete: "common.entryConflict.confirmKeepTheirsDeleteHeading",
      create: "common.entryConflict.confirmKeepTheirsCreateHeading",
    },
    keep_mine: {
      edit: "common.entryConflict.confirmKeepMineEditHeading",
      delete: "common.entryConflict.confirmDeleteHeading",
      create: "common.entryConflict.confirmKeepMineCreateHeading",
    },
  };
  return (byChoice[pendingChoice.value ?? "keep_mine"] ?? {})[op.value];
});
const confirmLine = computed(() => {
  const byChoice: Record<
    EntryConflictChoice,
    Record<EntryConflictOp, string>
  > = {
    keep_theirs: {
      edit: "common.entryConflict.confirmKeepTheirsLine1",
      delete: "common.entryConflict.confirmKeepTheirsDeleteLine1",
      create: "common.entryConflict.confirmKeepTheirsCreateLine1",
    },
    keep_mine: {
      edit: "common.entryConflict.confirmKeepMineEditLine1",
      delete: "common.entryConflict.confirmDeleteLine1",
      create: "common.entryConflict.confirmKeepMineCreateLine1",
    },
  };
  return (byChoice[pendingChoice.value ?? "keep_mine"] ?? {})[op.value];
});
// Confirm button label — choice + op. keep-theirs discards YOUR change and
// keeps theirs, so its wording is op-specific (delete/create reuse the step-1
// "keep theirs" labels); keep-mine is the overwrite/remove (edit/create
// "overwrite", delete "delete").
const confirmBtnIdle = computed(() => {
  if (pendingChoice.value === "keep_theirs") {
    return op.value === "delete"
      ? t("common.entryConflict.keepTheirsDelete")
      : op.value === "create"
        ? t("common.entryConflict.keepTheirsCreate")
        : t("common.entryConflict.discardMyEdit");
  }
  return op.value === "delete"
    ? t("common.entryConflict.deleteBtn")
    : t("common.entryConflict.overwrite");
});
const confirmBtnBusy = computed(() => {
  if (pendingChoice.value === "keep_theirs") {
    return op.value === "delete" || op.value === "create"
      ? t("common.entryConflict.keepingTheirs")
      : t("common.entryConflict.discarding");
  }
  return op.value === "delete"
    ? t("common.entryConflict.deleting")
    : t("common.entryConflict.overwriting");
});

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
 *  decrypt (recipient set changed); keep-theirs stays a stated leap of faith.
 *
 *  Tradeoff (R026): `show_password` runs `maybe_soft_wipe`, which under the
 *  default Immediate auto-lock wipes the identity cache the edit save deferred
 *  (so a keep-mine-edit resolve could reuse it without a second unlock). After a
 *  Preview, a keep-mine-edit resolve therefore re-prompts for unlock via
 *  `runWithAuth` — safe, just one extra prompt. Coupling the canonical reveal
 *  path to conflict-state to avoid it would be worse than the prompt. */
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
      <EntryAttributes :attributes="attributes ?? []" />
      <p v-if="clearsInSecs > 0" class="text-xs text-muted ec-clears">
        {{ t("common.entryConflict.autoClearsIn", { secs: clearsInSecs }) }}
      </p>
    </div>
    <p v-else-if="previewError" class="text-xs text-danger mb-2" role="alert">
      {{ previewError }}
    </p>
    <BaseButton
      v-else
      variant="link"
      tone="muted"
      size="xs"
      class="ec-preview-btn mb-2"
      :loading="previewLoading"
      :disabled="resolving"
      :aria-label="t('common.entryConflict.previewAriaLabel')"
      @click="previewTheirs"
    >
      {{ t("common.entryConflict.previewLabel") }}
    </BaseButton>

    <p v-if="error" class="text-xs text-danger mb-2" role="alert">
      {{ error }}
    </p>

    <div class="flex flex-col gap-2">
      <!-- Both step-1 choices are neutral outline — danger is reserved for the
           step-2 confirm (mirrors DivergenceModal: the sheet doesn't bias toward
           either path; the destructive action is re-confirmed in red). -->
      <BaseButton variant="outline" block @click="openConfirm('keep_theirs')">
        {{ keepTheirsLabel }}
      </BaseButton>
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
      {{ t(confirmHeading) }}
    </h2>

    <p class="text-sm mb-3">
      {{ t(confirmLine, { name: conflict!.name }) }}
    </p>

    <div class="flex flex-col gap-2">
      <BaseButton
        variant="danger"
        size="sm"
        :loading="resolving"
        @click="confirm"
      >
        {{ resolving ? confirmBtnBusy : confirmBtnIdle }}
      </BaseButton>
      <BaseButton size="sm" :disabled="resolving" @click="cancelConfirm">
        {{ t("common.button.cancel") }}
      </BaseButton>
    </div>
  </BaseModalShell>
</template>

<style scoped>
/* The divergence-block styles mirror DivergenceModal so the two conflict modals
   read as one family. The danger actions use BaseButton variant="danger"
   (DivergenceModal migrated off a bespoke .btn-danger — kept there too). */
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
</style>
