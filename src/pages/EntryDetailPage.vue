<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import {
  copyPassword as copyPasswordCmd,
  deleteSecret as deleteSecretCmd,
  showPassword as showPasswordCmd,
  type AppError,
  type DivergenceChoice,
  type PullResult,
} from "@/api";
import DivergenceModal from "@/components/DivergenceModal.vue";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseSpinner from "@/components/base/BaseSpinner.vue";
import {
  ensureClipboardNotifyPermission,
  isAuthCancelled,
  useDivergence,
  useLockState,
  useSecretReveal,
  useSecuritySettings,
  useToast,
} from "@/composables";
import { clipboardNotifyText } from "@/i18n/native";
import { navBack } from "@/utils/nav";
import { ArrowLeft, Copy, Eye } from "@lucide/vue";
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";

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

// Sensitive state lives in the shared secure-reveal composable: configurable
// auto-clear, wipe on unmount, wipe on browser back. `copyPassword` calls
// `clear()` itself.
const { password, notes, revealed, clearsInSecs, reveal, clear } =
  useSecretReveal();
const { viewClearSecs } = useSecuritySettings();
const loading = ref(false);
const error = ref("");
// True only while the alert shows a reveal decrypt failure, so the
// "check your age identity" hint can be gated locale-independently. Reset
// alongside `error` at the start of every action.
const decryptError = ref(false);
const deleting = ref(false);

// Delete divergence (the edit flow now lives on /edit/:path, which has its own
// useDivergence instance).
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
      openDivergence(preview);
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

function editEntry() {
  router.push({ name: "entryEdit", params: { pathMatch } });
}

function goBack() {
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
      @click="editEntry"
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
            ? t("entry.autoClearsIn", { secs: clearsInSecs })
            : t("entry.staysVisible")
        }}
      </p>
    </div>

    <!-- Divergence modal (delete-triggered — "save" wording) -->
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
