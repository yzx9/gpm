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
import BaseHeader from "@/components/base/BaseHeader.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import BaseSpinner from "@/components/base/BaseSpinner.vue";
import BaseTextarea from "@/components/base/BaseTextarea.vue";
import {
  isAuthCancelled,
  useCancellableSave,
  useDivergence,
  useLockState,
  useSecureClaim,
  useToast,
  useWipeOnLeave,
} from "@/composables";
import { currentLocale, loadBundle } from "@/i18n";
import { navBack } from "@/utils/nav";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter, type RouteLocationRaw } from "vue-router";

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

// Shared by the header Back button, the form Cancel (goBack), and Save-success
// so all three return to the same read view and can't drift apart. The
// divergence callbacks above go to the entries list instead, so they stay inline.
const BACK_FALLBACK: RouteLocationRaw = {
  name: "entry",
  params: { pathMatch },
};

const editPassword = ref("");
const editNotes = ref("");
// The reassembled body captured at load, for the no-op-save dirty-check.
const loadedBody = ref("");
const loading = ref(false);
const saving = ref(false);
const error = ref("");
const decryptError = ref(false);
// Set when the entry is a binary attachment — can't be round-tripped through
// the text editor without destroying it, so editing is blocked at the source.
const isAttachment = ref(false);
// Set when the entry holds non-UTF-8 bytes — editing its lossy text view and
// saving would corrupt the original bytes, so editing is blocked at the source.
const isNonUtf8 = ref(false);
const { cancelling, cancelSave } = useCancellableSave();

// R031: the edit form shows decrypted plaintext, so hold a screen-capture claim
// for the page's lifetime. `withClaim` raises FLAG_SECURE before loadBody fills
// the fields. `loadToken` discards a late load resolving after we left.
const { withClaim } = useSecureClaim();
let loadToken = 0;
useWipeOnLeave(() => {
  loadToken++;
});

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
    !isAttachment.value &&
    !isNonUtf8.value &&
    editBody.value.trim() !== "" &&
    editBody.value !== loadedBody.value,
);

async function loadBody() {
  loading.value = true;
  error.value = "";
  decryptError.value = false;
  const myToken = ++loadToken;
  try {
    // withClaim raises FLAG_SECURE before the decrypted body arrives; a late
    // load resolving after we left is discarded by the token; a failed acquire
    // returns null → abort (the per-op replacement for the old route-guard abort).
    const claimed = await withClaim(() =>
      runWithAuth(() => showPasswordCmd(entryPath)),
    );
    if (myToken !== loadToken) return;
    if (!claimed) {
      error.value = t("common.toast.secureScreenFailed");
      return;
    }
    if (claimed.attachment) {
      // An attachment can't be edited as text without destroying it (a
      // byte-compatible write is deferred — R067). Block at the source — covers the
      // detail page's pre-probe window and direct /edit deep-links alike.
      isAttachment.value = true;
      error.value = t("entry.attachmentEditDisabledHint");
      return;
    }
    if (claimed.edit_blocked === "nonUtf8") {
      // Non-UTF-8 bytes can't round-trip through a UTF-8 text editor without
      // corruption; block at the source so the lossy view is never saved back.
      isNonUtf8.value = true;
      error.value = t("entry.nonUtf8EditDisabledHint");
      return;
    }
    editPassword.value = claimed.password ?? "";
    editNotes.value = claimed.notes ?? "";
    loadedBody.value = reassemble(editPassword.value, editNotes.value);
  } catch (e) {
    if (isAuthCancelled(e)) return;
    const appError = e as AppError;
    decryptError.value = true;
    error.value = appError?.message || t("entry.decryptFailed");
    console.error("[entry-edit] decrypt failed", e);
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
      navBack(router, BACK_FALLBACK);
    } else if (outcome.kind === "needs_divergence_resolve") {
      // The edit's push lost a race — surface the divergence. The local edit was
      // committed; adopt discards it, keep pushes it. Stay on the edit form.
      const { kind: _kind, ...preview } = outcome;
      void _kind;
      openDivergence(preview);
    } else if (outcome.kind === "cancelled") {
      // User aborted. Nothing was published; if a commit was made it stays local
      // and syncs next time. Stay on the form — neutral status, not an error.
      toast.info(
        outcome.committed
          ? t("entry.saveCancelledLocalStays")
          : t("entry.saveCancelledNothingPublished"),
      );
    } else {
      // authenticity_blocked — pre-write pull refused under Enforce.
      error.value = t("entry.saveBlocked");
    }
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("entry.saveFailed");
    console.error("[entry-edit] save failed", e);
  } finally {
    saving.value = false;
    cancelling.value = false;
  }
}

function goBack() {
  navBack(router, BACK_FALLBACK);
}
</script>

<template>
  <main class="max-w-120 mx-auto p-4" role="main">
    <BaseHeader :back-fallback="BACK_FALLBACK">
      <template #title>
        <h1
          class="text-lg whitespace-nowrap overflow-hidden text-ellipsis flex-1"
        >
          {{ entryName }}
        </h1>
      </template>
    </BaseHeader>

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
          :disabled="cancelling"
          :aria-label="
            saving ? t('entry.cancelSaveAria') : t('entry.cancelEditAria')
          "
          @click="saving ? cancelSave() : goBack()"
        >
          {{
            cancelling
              ? t("entry.cancellingSave")
              : saving
                ? t("entry.cancelSave")
                : t("common.button.cancel")
          }}
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
