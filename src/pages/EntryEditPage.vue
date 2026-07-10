<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import {
  editSecret,
  showPassword as showPasswordCmd,
  type AppError,
  type DivergenceChoice,
  type PullResult,
} from "@/api";
import DivergenceModal from "@/components/DivergenceModal.vue";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import BaseSpinner from "@/components/base/BaseSpinner.vue";
import BaseTextarea from "@/components/base/BaseTextarea.vue";
import {
  isAuthCancelled,
  useDivergence,
  useLockState,
  useToast,
  useWipeOnLeave,
} from "@/composables";
import { currentLocale, loadBundle } from "@/i18n";
import { navBack } from "@/utils/nav";
import { ArrowLeft } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";

// The edit form reuses the `entry.*` bundle (loaded for the read view); load it
// explicitly so a cold deep-link to /edit/… resolves keys without a prior /entry visit.
void loadBundle(currentLocale(), "entry");

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const { runWithAuth } = useLockState();
const { toast } = useToast();

const pathMatch = route.params.pathMatch;
const entryPath = decodeURIComponent(
  Array.isArray(pathMatch) ? pathMatch[0] : pathMatch,
);
const entryName = entryPath.replace(/\.age$/, "");

const editPassword = ref("");
const editNotes = ref("");
// The reassembled body captured at load, for the no-op-save dirty-check.
const loadedBody = ref("");
const loading = ref(false);
const saving = ref(false);
const error = ref("");
const decryptError = ref(false);

const {
  divergence,
  resolving,
  divergeError,
  openDivergence,
  resolveDivergence,
  cancelDivergence,
} = useDivergence({
  resolveFailedKey: "entry.resolveFailed",
  onResolved(result: PullResult, choice: DivergenceChoice) {
    exitEdit();
    if (choice === "adopt_remote") {
      toast.info(t("entry.adoptedRemote"));
    } else {
      toast.success(t("entry.keptMine", { head: result.head }));
    }
    navBack(router, { name: "entries" });
  },
  onPullFfFailed() {
    toast.info(t("entry.remoteChanged"));
    navBack(router, { name: "entries" });
  },
});

onMounted(loadBody);

/** Reassemble the edit body to match `Secret::parse`: first line is the password,
 *  the rest is notes. NO trim — `Secret::parse` doesn't trim the password, so
 *  trimming would silently change a secret with whitespace. Lossless inverse. */
function reassemble(pw: string, body: string): string {
  return body ? `${pw}\n${body}` : pw;
}

const editBody = computed(() =>
  reassemble(editPassword.value, editNotes.value),
);

/** Save is enabled only when the body has non-whitespace content and actually
 *  changed. age ciphertext is non-deterministic, so an unchanged Save would
 *  still make a spurious commit (block it); and an all-whitespace body would be
 *  rejected by `Secret::parse` on the next read, bricking the secret (block it).
 *  The trim is on the GATE only — the saved body stays untrimmed (lossless). */
const canSave = computed(
  () =>
    !saving.value &&
    editBody.value.trim() !== "" &&
    editBody.value !== loadedBody.value,
);

async function loadBody() {
  loading.value = true;
  error.value = "";
  decryptError.value = false;
  try {
    const result = await runWithAuth(() => showPasswordCmd(entryPath));
    editPassword.value = result.password ?? "";
    editNotes.value = result.notes ?? "";
    loadedBody.value = reassemble(editPassword.value, editNotes.value);
  } catch (e) {
    if (isAuthCancelled(e)) return;
    const appError = e as AppError;
    decryptError.value = true;
    error.value = appError?.message || t("entry.decryptFailed");
  } finally {
    loading.value = false;
  }
}

function exitEdit() {
  editPassword.value = "";
  editNotes.value = "";
  loadedBody.value = "";
}

// Wipe the working plaintext on browser back, unmount, and hard lock so it
// doesn't survive behind a wiped identity. (useDivergence clears its own
// modal state on lock.)
useWipeOnLeave(exitEdit);

async function onSave() {
  if (!canSave.value) return;
  saving.value = true;
  error.value = "";
  decryptError.value = false;
  try {
    const outcome = await editSecret(entryName, editBody.value);
    if (outcome.kind === "written") {
      toast.success(t("entry.saved", { commit: outcome.commit }));
      // Back to the read view (the opener) — it remounts and shows fresh content.
      navBack(router, { name: "entry", params: { pathMatch } });
    } else if (outcome.kind === "needs_divergence_resolve") {
      // The edit's push lost a race — surface the divergence. The local edit was
      // committed; adopt discards it, keep pushes it. Stay on the edit form.
      const { kind: _kind, ...preview } = outcome;
      void _kind;
      openDivergence(preview);
    } else {
      // authenticity_blocked — pre-write pull refused under Enforce.
      error.value = t("entry.saveBlocked");
    }
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("entry.saveFailed");
  } finally {
    saving.value = false;
  }
}

function goBack() {
  navBack(router, { name: "entry", params: { pathMatch } });
}
</script>

<template>
  <main class="max-w-120 mx-auto p-4" role="main">
    <header class="flex items-center gap-3 mb-6" role="banner">
      <button
        @click="goBack"
        class="bg-transparent border-none text-base cursor-pointer text-accent active:text-accent-deep p-1 min-w-12 min-h-12 inline-flex items-center gap-1"
        :aria-label="t('common.back')"
      >
        <BaseIcon :icon="ArrowLeft" /> {{ t("common.back") }}
      </button>
      <h1
        class="text-lg whitespace-nowrap overflow-hidden text-ellipsis flex-1"
      >
        {{ entryName }}
      </h1>
    </header>

    <BaseAlert v-if="error" variant="danger" class="mb-4">
      {{ error }}
      <span v-if="decryptError" class="block text-xs opacity-80 mt-1">
        {{ t("entry.checkIdentityHint") }}
      </span>
    </BaseAlert>

    <div
      v-if="loading"
      class="flex items-center justify-center gap-2 text-center text-muted py-4"
    >
      <BaseSpinner />
      <span>{{ t("common.loading") }}</span>
    </div>

    <form v-else class="flex flex-col gap-4 mb-6" @submit.prevent="onSave">
      <div class="flex flex-col gap-1">
        <label for="e-password" class="text-sm font-medium">{{
          t("entry.password")
        }}</label>
        <BaseInput
          id="e-password"
          v-model="editPassword"
          type="text"
          class="font-mono"
          autocomplete="off"
          spellcheck="false"
        />
      </div>
      <div class="flex flex-col gap-1">
        <label for="e-notes" class="text-sm font-medium">{{
          t("entry.notes")
        }}</label>
        <BaseTextarea
          id="e-notes"
          v-model="editNotes"
          rows="6"
          autocomplete="off"
        />
        <small class="text-xs text-muted">{{ t("entry.firstLineHint") }}</small>
      </div>
      <div class="flex gap-3">
        <BaseButton
          variant="primary"
          type="submit"
          class="flex-1"
          :disabled="!canSave"
          :aria-label="t('entry.saveAria')"
        >
          {{ saving ? t("entry.saving") : t("entry.saveLabel") }}
        </BaseButton>
        <BaseButton
          variant="outline"
          type="button"
          class="flex-1"
          :disabled="saving"
          :aria-label="t('entry.cancelEditAria')"
          @click="goBack"
        >
          {{ t("common.button.cancel") }}
        </BaseButton>
      </div>
    </form>

    <!-- Divergence modal (save-triggered — "save" wording) -->
    <DivergenceModal
      context="save"
      :divergence="divergence"
      :resolving="resolving"
      :error="divergeError"
      @resolve="resolveDivergence"
      @close="cancelDivergence"
    />
  </main>
</template>
