<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import {
  copyPassword as copyPasswordCmd,
  deleteSecret as deleteSecretCmd,
  discardDivergence,
  editSecret,
  resolveSyncDivergence,
  showPassword as showPasswordCmd,
  type AppError,
  type DivergenceChoice,
  type PullResult,
  type SyncDivergence,
} from "@/api";
import DivergenceModal from "@/components/DivergenceModal.vue";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import BaseSpinner from "@/components/base/BaseSpinner.vue";
import BaseTextarea from "@/components/base/BaseTextarea.vue";
import {
  ensureClipboardNotifyPermission,
  isAuthCancelled,
  useLockState,
  useSecretReveal,
  useSecuritySettings,
  useToast,
} from "@/composables";
import { clipboardNotifyText } from "@/i18n/native";
import { navBack } from "@/utils/nav";
import { ArrowLeft, Copy, Eye } from "@lucide/vue";
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const { onLock, runWithAuth } = useLockState();
const { toast } = useToast();

const entryPath = decodeURIComponent(
  Array.isArray(route.params.pathMatch)
    ? route.params.pathMatch[0]
    : route.params.pathMatch,
);
const entryName = entryPath.replace(/\.age$/, "");

// Sensitive state lives in the shared secure-reveal composable: configurable
// auto-clear, wipe on unmount, wipe on browser back. `copyPassword` calls
// `clear()` itself.
const { password, notes, revealed, reveal, clear } = useSecretReveal();
const { viewClearSecs } = useSecuritySettings();
const loading = ref(false);
const error = ref("");
// True only while the alert shows a reveal/edit decrypt failure, so the
// "check your age identity" hint can be gated locale-independently (the old
// `error.includes("ecrypt")` substring match broke once the fallback string
// was translated). Reset alongside `error` at the start of every action and
// set in the two decrypt-path catches.
const decryptError = ref(false);
const deleting = ref(false);

// ── Edit mode ──────────────────────────────────────────────────────────────
// Edit holds the working plaintext in plain refs (not the reveal composable) so
// it can stay armed while editing without a 30s auto-clear firing mid-edit. To
// keep a single plaintext copy, entering edit drops the reveal buffer; the refs
// are wiped on save/cancel and on lock.
const editing = ref(false);
const saving = ref(false);
const editPassword = ref("");
const editNotes = ref("");
// The reassembled body captured at edit-entry, for the no-op-save dirty-check.
const loadedBody = ref("");

// ── Divergence (edit/delete collides with a newer remote) ─────────────────
const divergence = ref<SyncDivergence | null>(null);
const resolving = ref(false);
const divergeError = ref("");

// Wipe edit plaintext on lock so it doesn't survive the 5-min auto-lock behind a
// wiped identity (mirrors CreatePage's onLock wipe of the compose buffer).
onLock(() => {
  exitEdit();
  divergence.value = null;
});

async function showPassword() {
  // Toggle off: if already revealed, hide (and wipe the plaintext) instead of
  // re-running auth + decrypt. clear() cancels the auto-clear timer too.
  if (revealed.value) {
    clear();
    return;
  }
  loading.value = true;
  error.value = "";
  decryptError.value = false;
  try {
    const result = await runWithAuth(() => showPasswordCmd(entryPath));
    reveal(result);
  } catch (e) {
    if (isAuthCancelled(e)) return;
    const appError = e as AppError;
    decryptError.value = true;
    error.value = appError?.message || t("entry.decryptFailed");
  } finally {
    loading.value = false;
  }
}

async function copyPassword() {
  error.value = "";
  decryptError.value = false;
  try {
    await ensureClipboardNotifyPermission();
    const result = await runWithAuth(() =>
      copyPasswordCmd(entryPath, clipboardNotifyText()),
    );
    clear();
    toast.success(
      t("entry.copied", {
        name: result.entry_name,
        secs: result.cleared_after_secs,
      }),
    );
  } catch (e) {
    if (isAuthCancelled(e)) return;
    const appError = e as AppError;
    error.value = appError?.message || t("common.toast.copyFailed");
  }
}

async function deleteSecret() {
  if (deleting.value) return;
  if (!confirm(t("entry.deleteConfirm", { name: entryName }))) {
    return;
  }
  deleting.value = true;
  error.value = "";
  decryptError.value = false;
  try {
    const outcome = await deleteSecretCmd(entryName);
    if (outcome.kind === "written") {
      clear();
      toast.success(t("entry.deleted", { commit: outcome.commit }));
      // Pop to entries (the opener). The deleted-entry page becomes forward
      // history, which Android system back can't reopen.
      navBack(router, { name: "entries" });
    } else if (outcome.kind === "needs_divergence_resolve") {
      // The delete's push lost a race — surface the divergence. The local delete
      // was committed; adopt discards it (entry returns), keep pushes it.
      const { kind: _kind, ...preview } = outcome;
      void _kind;
      divergence.value = preview;
      divergeError.value = "";
    } else {
      // authenticity_blocked — pre-write pull refused under Enforce.
      error.value = t("entry.deleteBlocked");
    }
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("entry.deleteFailed");
  } finally {
    deleting.value = false;
  }
}

/** Reassemble the edit body to match Secret::parse: first line is the password,
 *  the rest is notes. NO trim — Secret::parse doesn't trim the password, so
 *  trimming would silently change a secret with whitespace. Lossless inverse. */
function reassemble(pw: string, body: string): string {
  return body ? `${pw}\n${body}` : pw;
}

/** The body assembled from the current edit fields (drives the dirty-check). */
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

/** Enter edit mode. Cold-edit safe: if the user never clicked Show, fetch the
 *  content first so the fields can prefill. Drops the reveal buffer so only one
 *  plaintext copy is live. */
async function enterEdit() {
  if (editing.value) return;
  error.value = "";
  decryptError.value = false;
  let pw = password.value;
  let nt = notes.value;
  if (pw === null) {
    // Cold edit — the page never revealed; fetch so the form can prefill.
    loading.value = true;
    try {
      const result = await runWithAuth(() => showPasswordCmd(entryPath));
      pw = result.password;
      nt = result.notes;
    } catch (e) {
      if (isAuthCancelled(e)) return;
      const appError = e as AppError;
      decryptError.value = true;
      error.value = appError?.message || t("entry.decryptFailed");
      return;
    } finally {
      loading.value = false;
    }
  }
  editPassword.value = pw ?? "";
  editNotes.value = nt ?? "";
  loadedBody.value = reassemble(editPassword.value, editNotes.value);
  // Single plaintext copy: drop the read-path reveal buffer.
  clear();
  editing.value = true;
}

function exitEdit() {
  editPassword.value = "";
  editNotes.value = "";
  loadedBody.value = "";
  editing.value = false;
}

function cancelEdit() {
  exitEdit();
}

async function saveEdit() {
  if (!canSave.value) return;
  saving.value = true;
  error.value = "";
  decryptError.value = false;
  try {
    const outcome = await editSecret(entryName, editBody.value);
    if (outcome.kind === "written") {
      toast.success(t("entry.saved", { commit: outcome.commit }));
      // Exit to the read-only view; the user can Show to verify. Don't
      // auto-reveal the password post-save.
      exitEdit();
    } else if (outcome.kind === "needs_divergence_resolve") {
      // The edit's push lost a race — surface the divergence. The local edit was
      // committed; adopt discards it, keep pushes it. Stay in edit mode.
      const { kind: _kind, ...preview } = outcome;
      void _kind;
      divergence.value = preview;
      divergeError.value = "";
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

/** The user dismissed the edit/delete divergence modal (cancel / back). The
 *  local commit (edit or delete) stays and publishes on the next Sync; clear the
 *  identity the save path kept alive (deferred wipe) for a possible keep-mine
 *  resolve. Fire-and-forget. */
function cancelDivergence() {
  if (!divergence.value) return;
  divergence.value = null;
  divergeError.value = "";
  void discardDivergence().catch(() => {});
}

/** Resolve the edit/delete divergence per the user's choice. `keep_mine`
 *  re-encrypts local-only entries onto the reviewed remote tip and pushes —
 *  needs the identity (runWithAuth). `adopt_remote` is a fast-forward. */
async function resolveDivergence(choice: DivergenceChoice) {
  if (!divergence.value) return;
  resolving.value = true;
  divergeError.value = "";
  const expectedRemoteOid = divergence.value.remote_tip;
  try {
    const result: PullResult =
      choice === "keep_mine"
        ? await runWithAuth(() =>
            resolveSyncDivergence(expectedRemoteOid, choice),
          )
        : await resolveSyncDivergence(expectedRemoteOid, choice);
    divergence.value = null;
    exitEdit();
    if (choice === "adopt_remote") {
      toast.info(t("entry.adoptedRemote"));
    } else {
      toast.success(t("entry.keptMine", { head: result.head }));
    }
    // Pop to entries; the resolved-divergence UI becomes forward history.
    navBack(router, { name: "entries" });
  } catch (e) {
    if (isAuthCancelled(e)) return;
    const appError = e as AppError;
    if (appError?.code === "PULL_FF_FAILED") {
      divergence.value = null;
      toast.info(t("entry.remoteChanged"));
      navBack(router, { name: "entries" });
    } else {
      divergeError.value = appError?.message || t("entry.resolveFailed");
    }
  } finally {
    resolving.value = false;
  }
}

function goBack() {
  // In edit mode, Back/Escape exits edit (keeps the user on the page) instead of
  // navigating away and silently dropping the draft.
  if (editing.value) {
    cancelEdit();
    return;
  }
  clear();
  // Pop to the page that opened this entry (normally entries). At a deep-link
  // root there's nothing to pop, so fall back to entries as the new root.
  navBack(router, { name: "entries" });
}

function handleKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") {
    goBack();
  }
}
</script>

<template>
  <main class="max-w-120 mx-auto p-4" role="main" @keydown="handleKeydown">
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
    <!-- Read-only view -->
    <div v-if="!editing">
      <div class="flex gap-3 mb-6">
        <BaseButton
          variant="primary"
          class="flex-1"
          :disabled="loading || deleting"
          :aria-label="t('entry.copyAria')"
          @click="copyPassword"
        >
          <BaseIcon :icon="Copy" /> {{ t("entry.copyLabel") }}
        </BaseButton>
        <BaseButton
          variant="outline"
          class="flex-1"
          :disabled="loading || deleting"
          :aria-label="
            revealed ? t('entry.showingAria') : t('entry.showPasswordAria')
          "
          @click="showPassword"
        >
          <BaseIcon :icon="Eye" />
          {{ revealed ? t("entry.showingLabel") : t("entry.showLabel") }}
        </BaseButton>
      </div>

      <BaseButton
        variant="outline"
        block
        class="mb-3"
        :disabled="loading || deleting"
        :aria-label="t('entry.editAria', { name: entryName })"
        @click="enterEdit"
      >
        {{ t("entry.editLabel") }}
      </BaseButton>

      <BaseButton
        variant="danger"
        block
        class="mb-6"
        :disabled="deleting || loading"
        :aria-label="t('entry.deleteAria', { name: entryName })"
        @click="deleteSecret"
      >
        {{ deleting ? t("entry.deleting") : t("entry.deleteLabel") }}
      </BaseButton>

      <div
        v-if="loading"
        class="flex items-center justify-center gap-2 text-center text-muted py-4"
      >
        <BaseSpinner />
        <span>{{ t("entry.decrypting") }}</span>
      </div>

      <div
        v-if="revealed && password !== null"
        class="bg-surface rounded-lg p-4 shadow-[0_1px_6px_rgba(0,0,0,0.06)]"
      >
        <div class="mb-4">
          <label
            class="block text-xs font-semibold uppercase tracking-wide text-muted mb-1"
            >{{ t("entry.password") }}</label
          >
          <div
            class="font-mono text-lg p-2 bg-accent-ring rounded-sm break-all select-all"
          >
            {{ password }}
          </div>
        </div>

        <div v-if="notes" class="mb-2">
          <label
            class="block text-xs font-semibold uppercase tracking-wide text-muted mb-1"
            >{{ t("entry.notes") }}</label
          >

          <!-- prettier-ignore -->
          <pre
            class="text-sm p-2 bg-input rounded-sm whitespace-pre-wrap break-all font-[inherit] select-text max-h-50 overflow-y-auto"
            >{{ notes }}</pre>
        </div>

        <p class="text-center text-xs text-muted mt-3">
          {{
            viewClearSecs > 0
              ? t("entry.autoClearsIn", { secs: viewClearSecs })
              : t("entry.staysVisible")
          }}
        </p>
      </div>
    </div>

    <!-- Edit form -->
    <form v-else class="flex flex-col gap-4 mb-6" @submit.prevent="saveEdit">
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
          @click="cancelEdit"
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

    <!-- Full repository path, shown as quiet footer metadata -->
    <p class="text-center text-xs text-muted break-all select-all mt-8">
      {{ entryPath }}
    </p>
  </main>
</template>
