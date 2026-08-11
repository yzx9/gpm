<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<script setup lang="ts">
import {
  copyPassword as copyPasswordCmd,
  copyTotp as copyTotpCmd,
  deleteSecret as deleteSecretCmd,
  ensureClipboardNotifyPermission,
  entryOid as entryOidCmd,
  entryProbe as entryProbeCmd,
  exportAttachment as exportAttachmentCmd,
  showPassword as showPasswordCmd,
  wipeEntryCache,
  type AppError,
  type AttachmentMeta,
  type DivergenceChoice,
  type EditBlockReason,
  type EntryConflictChoice,
  type EntryConflictOp,
  type PullResult,
} from "@/api";
import DivergenceModal from "@/components/DivergenceModal.vue";
import EntryAttributes from "@/components/EntryAttributes.vue";
import EntryConflictModal from "@/components/EntryConflictModal.vue";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseHeader from "@/components/base/BaseHeader.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseSpinner from "@/components/base/BaseSpinner.vue";
import {
  isAuthCancelled,
  useActiveRepo,
  useCancellableSave,
  useDialog,
  useDivergence,
  useEntryCacheState,
  useEntryConflict,
  useLockState,
  useSecretReveal,
  useSecuritySettings,
  useToast,
  useWipeOnLeave,
} from "@/composables";
import { clipboardNotifyText } from "@/i18n/native";
import { navBack } from "@/utils/nav";
import { Clock, Copy, Download, Eye, Lock, Paperclip } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter, type RouteLocationRaw } from "vue-router";

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const { runWithAuth, identityCached } = useLockState();
// R086: per-view mirror of the backend entry-view cache. Cold (not cached, and
// identity not cached either) ⇒ the Unlock gate; warm ⇒ the full button set, and
// actions skip the per-op auth gate (the cache serves without re-prompting).
const { entryCached } = useEntryCacheState();
const activeRepo = useActiveRepo();
const { toast } = useToast();
const { dialog } = useDialog();

const pathMatch = route.params.pathMatch;
const entryPath = decodeURIComponent(
  Array.isArray(pathMatch) ? pathMatch[0] : pathMatch,
);
const entryName = entryPath.replace(/\.age$/, "");

// Sensitive state lives in the shared secure-reveal composable: configurable
// auto-clear, wipe on unmount, wipe on browser back. `copyPassword` calls
// `clear()` itself. `withClaim` raises FLAG_SECURE before the decrypted secret
// arrives and brands it so `reveal()` can type-check (R031).
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
const { viewClearSecs } = useSecuritySettings();
// Invalidation token (R031): a decrypt resolving after we left (Back/lock) must
// not write the secret into a leaving/dead component. Bumped on every leave.
let revealToken = 0;
useWipeOnLeave(() => {
  revealToken++;
  // R086: wipe the entry-view cache on leave/switch (popstate / unmount /
  // either lock) so the just-left entry's decrypted content does not linger in
  // backend memory. Best-effort + idempotent — must not throw (useWipeOnLeave
  // contract).
  void wipeEntryCache(entryPath).catch(() => {});
});
const loading = ref(false);
const error = ref("");
// True only while the alert shows a reveal decrypt failure, so the
// "check your age identity" hint can be gated locale-independently. Reset
// alongside `error` at the start of every action.
const decryptError = ref(false);
const deleting = ref(false);
// R026: the blob oid probed on mount (the base version) — sent on delete so a
// stale delete surfaces entry_conflict instead of silently removing a teammate's
// newer version. Non-secret; decoupled from reveal (delete-without-reveal is
// still protected).
const baseOid = ref<string | null>(null);
const { cancelling, cancelSave } = useCancellableSave();

// Entry affordance signals, all tri-state: `null` = unknown (not yet probed /
// identity not cached), `true`/`false` once we know. The probe runs only when
// the identity is already cached — so it never triggers an unlock. Under a
// per-op lock the buttons stay as fallbacks until the first copy/show/export on
// this entry reports the truth back via its result.
const showTotp = ref<boolean | null>(null);
const showAttachment = ref<boolean | null>(null);
const attachmentMeta = ref<AttachmentMeta | null>(null);
// Why Edit is disabled, if the probe found a reason (e.g. non-UTF-8 content).
const editBlockedReason = ref<EditBlockReason | null>(null);
const probing = ref(false);

// A confirmed attachment restructures the page: the password actions are dead
// (empty password, base64 body), so Export becomes the primary affordance and
// Edit locks. While attachment status is unknown (locked), the password actions
// stay visible as the familiar fallback and Export also shows so the attachment
// stays discoverable; once confirmed either way the layout collapses to the
// relevant set.
const isAttachment = computed(() => showAttachment.value === true);
const passwordActionsVisible = computed(() => !isAttachment.value);
const exportButtonVisible = computed(() => showAttachment.value !== false);
const editDisabled = computed(
  () => isAttachment.value || editBlockedReason.value !== null,
);
const totpButtonVisible = computed(() => {
  if (isAttachment.value) return false; // attachments carry no TOTP
  if (showTotp.value === false) return false;
  // Hold the button while a free (cached) probe is in flight to avoid a flash.
  return !probing.value;
});

// R086: the cold-start gate shows when neither the entry cache nor the identity
// is warm (we'd need a prompt). Once either is warm the buttons show: the entry
// cache serves from a HIT (identity soft-wiped but content cached), or the
// identity cache decrypts on a MISS without a prompt. This also avoids a gate
// flash when the identity is already cached on mount (the probe warms in the
// background).
const gateVisible = computed(() => !entryCached.value && !identityCached.value);
const unlocking = ref(false);

/** Run a secret action, skipping the per-op auth gate when the entry cache or
 *  identity is warm (R086) — the backend serves from the cache (or decrypts with
 *  the cached identity) without re-prompting. Cold (a race the gate normally
 *  prevents) falls back to `runWithAuth`. */
async function runEntryAction<T>(fn: () => Promise<T>): Promise<T> {
  if (entryCached.value || identityCached.value) {
    try {
      return await fn();
    } catch (e) {
      // The warm snapshot was stale — the cache was evicted under us (view-clear
      // timer, a hard lock, or a background-sync oid change forcing a MISS). Fall
      // back to the auth-gated path so the user gets the unlock overlay, not a raw
      // IDENTITY_ENCRYPTED error (the race the old runWithAuth loop parked for).
      if ((e as AppError)?.code === "IDENTITY_ENCRYPTED") {
        return runWithAuth(fn);
      }
      throw e;
    }
  }
  return runWithAuth(fn);
}

/** The gate's single action: probe (decrypt once + warm the cache) behind auth.
 *  A `null` probe after `runWithAuth` is the Immediate soft-wipe race (the
 *  identity was wiped between the unlock and the decrypt) — retry once. A second
 *  `null` stays on the gate with an error (D7; no infinite loop). */
async function unlockEntry() {
  if (unlocking.value) return;
  unlocking.value = true;
  error.value = "";
  decryptError.value = false;
  try {
    let probe = await runWithAuth(() => entryProbeCmd(entryPath));
    if (probe === null) {
      // Immediate soft-wipe race — the unlock succeeded but the identity was
      // wiped before the probe's decrypt. Retry once.
      probe = await runWithAuth(() => entryProbeCmd(entryPath));
    }
    if (probe === null) {
      error.value = t("entry.unlockFailed");
      return;
    }
    // Cache warmed — the `entry-cache-warmed` event flips `entryCached` and the
    // gate swaps to the buttons. Settle the affordances from the probe.
    showTotp.value = probe.has_totp;
    showAttachment.value = probe.attachment !== null;
    attachmentMeta.value = probe.attachment;
    editBlockedReason.value = probe.edit_blocked;
  } catch (e) {
    if (isAuthCancelled(e)) return;
    const appError = e as AppError;
    decryptError.value = true;
    error.value = appError?.message || t("entry.unlockFailed");
    console.error("[entry-detail] unlock failed", e);
  } finally {
    unlocking.value = false;
  }
}

async function probeEntry() {
  if (!identityCached.value || showAttachment.value !== null || probing.value) {
    return;
  }
  probing.value = true;
  try {
    const repoId = await activeRepo.currentId();
    const probe = await entryProbeCmd(repoId, entryPath);
    if (probe !== null) {
      showTotp.value = probe.has_totp;
      showAttachment.value = probe.attachment !== null;
      attachmentMeta.value = probe.attachment;
      editBlockedReason.value = probe.edit_blocked;
    }
  } catch (e) {
    // Probe failed (rare) — leave unknown; buttons stay as fallbacks.
    console.debug("[entry-detail] probe failed", e);
  } finally {
    probing.value = false;
  }
}

// Probe on open when the identity is cached (free — no unlock). When it isn't,
// the buttons stay as fallbacks until the first action reports the truth back.
onMounted(probeEntry);

// Humanize a byte count for the attachment metadata caption (B/KB/MB/GB).
function humanizeSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value.toFixed(value < 10 ? 1 : 0)} ${units[unit]}`;
}

// R026: capture the entry's base version on mount for a base-version-aware
// delete. Non-secret (no identity, no decrypt), so it runs unconditionally — a
// delete-without-reveal stays protected.
async function probeVersion() {
  try {
    const repoId = await activeRepo.currentId();
    baseOid.value = await entryOidCmd(repoId, entryPath);
  } catch {
    // Probe failed (rare) — leave null (legacy unprotected delete path).
  }
}
onMounted(probeVersion);

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

const {
  conflict: entryConflict,
  resolving: entryConflictResolving,
  conflictError: entryConflictError,
  openConflict,
  resolveConflict,
  cancelConflict,
} = useEntryConflict({
  resolveFailedKey: "entry.resolveFailed",
  onResolved(
    result: PullResult,
    choice: EntryConflictChoice,
    op: EntryConflictOp,
  ) {
    if (op === "delete") {
      if (choice === "keep_mine") {
        clear();
        toast.success(t("entry.deleted", { commit: result.head }));
      } else {
        toast.info(t("entry.conflictKeptTheirs"));
      }
    }
    navBack(router, { name: "entries" });
  },
  onPullFfFailed() {
    toast.info(t("entry.remoteChanged"));
    navBack(router, { name: "entries" });
  },
  onAuthenticityBlocked() {
    // Enforce refused the resolve's re-fetch — nothing deleted. Stay + explain
    // (mirrors the save path's authenticity_blocked branch).
    error.value = t("entry.deleteBlocked");
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
  const myToken = ++revealToken;
  try {
    const repoId = await activeRepo.currentId();
    // withClaim raises FLAG_SECURE before the secret arrives and brands it; if
    // we left mid-decrypt (token bumped), discard rather than render into a
    // leaving/dead component. A failed acquire returns null → abort with a toast
    // (the per-op replacement for the old route-guard abort).
    const claimed = await withClaim(() =>
      runEntryAction(() => showPasswordCmd(repoId, entryPath)),
    );
    if (myToken !== revealToken) return;
    if (!claimed) {
      error.value = t("common.toast.secureScreenFailed");
      return;
    }
    reveal(claimed);
    showTotp.value = claimed.has_totp;
    showAttachment.value = claimed.attachment !== null;
    attachmentMeta.value = claimed.attachment;
    // Refresh the probe-derived affordance from the decrypted truth: a
    // mount-time probe miss (identity was locked) or a sync that changed the
    // entry (e.g. a YAML→AKV edit by a teammate) would otherwise leave Edit
    // stuck disabled — or enabled — until the page remounts.
    editBlockedReason.value = claimed.edit_blocked;
  } catch (e) {
    if (isAuthCancelled(e)) return;
    const appError = e as AppError;
    decryptError.value = true;
    error.value = appError?.message || t("entry.decryptFailed");
    console.error("[entry-detail] reveal failed", e);
  } finally {
    loading.value = false;
  }
}

async function copyPassword() {
  error.value = "";
  decryptError.value = false;
  try {
    await ensureClipboardNotifyPermission();
    const repoId = await activeRepo.currentId();
    const result = await runEntryAction(() =>
      copyPasswordCmd(repoId, entryPath, clipboardNotifyText()),
    );
    showTotp.value = result.has_totp;
    showAttachment.value = result.has_attachment;
    clear();
    if (result.has_attachment) {
      // Backend skipped the clipboard write (no password on an attachment).
      toast.info(t("entry.attachmentCopyBlocked"));
      return;
    }
    if (result.password_non_utf8) {
      // Backend skipped the clipboard write: the password has non-UTF-8 bytes
      // that can't go on the (UTF-8) clipboard and can't be shown or edited.
      toast.info(t("entry.nonUtf8CopyBlocked"));
      return;
    }
    if (result.password_empty) {
      // Backend skipped the clipboard write: a bare legacy-YAML document has
      // no password line — nothing to copy, don't toast a fake success.
      toast.info(t("entry.emptyPasswordCopyBlocked"));
      return;
    }
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
    console.error("[entry-detail] copy password failed", e);
  }
}

async function copyTotp() {
  error.value = "";
  decryptError.value = false;
  try {
    await ensureClipboardNotifyPermission();
    const repoId = await activeRepo.currentId();
    const result = await runEntryAction(() =>
      copyTotpCmd(repoId, entryPath, clipboardNotifyText()),
    );
    // `copied` is false exactly when the entry has no TOTP seed — reuse it as
    // the presence signal so the button settles to the right state after a tap.
    showTotp.value = result.copied;
    if (result.copied) {
      toast.success(
        t("entry.totpCopied", {
          name: result.entry_name,
          secs: result.cleared_after_secs,
        }),
      );
    } else {
      // No TOTP seed in this entry — gentle info, not an error.
      toast.info(t("entry.noTotp"));
    }
  } catch (e) {
    if (isAuthCancelled(e)) return;
    const appError = e as AppError;
    error.value = appError?.message || t("common.toast.copyFailed");
    console.error("[entry-detail] copy totp failed", e);
  }
}

async function exportAttachment() {
  error.value = "";
  decryptError.value = false;
  loading.value = true;
  try {
    const repoId = await activeRepo.currentId();
    const result = await runEntryAction(() =>
      exportAttachmentCmd(repoId, entryPath),
    );
    if (!result.exported) {
      // The entry holds no attachment (the button was a fallback) — settle the
      // signal and tell the user; no file was written.
      showAttachment.value = false;
      attachmentMeta.value = null;
      toast.info(t("entry.noAttachment"));
      return;
    }
    showAttachment.value = true;
    toast.success(t("entry.attachmentExported", { name: result.entry_name }));
  } catch (e) {
    if (isAuthCancelled(e)) return;
    const appError = e as AppError;
    // Dismissed save picker, or a second export already in flight — silent/benign.
    if (appError?.code === "CANCELLED") return;
    if (appError?.code === "REPO_BUSY") {
      toast.info(t("entry.attachmentExportBusy"));
      return;
    }
    error.value = appError?.message || t("entry.attachmentExportFailed");
    console.error("[entry-detail] export attachment failed", e);
  } finally {
    loading.value = false;
  }
}

async function deleteSecret() {
  if (deleting.value) return;
  const confirmed = await dialog.confirm({
    message: t("entry.deleteConfirm", { name: entryName }),
    confirmLabel: t("common.button.delete"),
    danger: true,
  });
  if (!confirmed) return;
  deleting.value = true;
  error.value = "";
  decryptError.value = false;
  try {
    const repoId = await activeRepo.currentId();
    const outcome = await deleteSecretCmd(repoId, entryName, baseOid.value);
    if (outcome.kind === "written") {
      clear();
      toast.success(t("entry.deleted", { commit: outcome.commit }));
      // Pop to entries (the opener). The deleted-entry page becomes forward
      // history, which Android system back can't reopen.
      navBack(router, { name: "entries" });
    } else if (outcome.kind === "no_change") {
      // a teammate already removed it — nothing to commit. Distinct
      // from `written` so we toast "already removed", not a fake delete commit.
      clear();
      toast.info(t("entry.alreadyRemoved"));
      navBack(router, { name: "entries" });
    } else if (outcome.kind === "entry_conflict") {
      // R026: the entry changed on the remote since the read — refuse the stale
      // delete and let the user pick. No plaintext to carry (delete).
      const { kind: _kind, ...payload } = outcome;
      void _kind;
      openConflict(payload, null);
    } else if (outcome.kind === "needs_divergence_resolve") {
      // The delete's push lost a race — surface the divergence. The local delete
      // was committed; adopt discards it (entry returns), keep pushes it.
      const { kind: _kind, ...preview } = outcome;
      void _kind;
      openDivergence(preview);
    } else if (outcome.kind === "cancelled") {
      // User aborted. Nothing was published; if committed, the local delete stays
      // and syncs next time. Stay on the detail page — neutral status, not error.
      toast.info(
        outcome.committed
          ? t("entry.deleteCancelledLocalStays")
          : t("entry.deleteCancelledNothingPublished"),
      );
    } else {
      // authenticity_blocked — pre-write pull refused under Enforce.
      error.value = t("entry.deleteBlocked");
    }
  } catch (e) {
    if (isAuthCancelled(e)) return;
    const appError = e as AppError;
    error.value = appError?.message || t("entry.deleteFailed");
    console.error("[entry-detail] delete failed", e);
  } finally {
    deleting.value = false;
    cancelling.value = false;
  }
}

function editEntry() {
  router.push({ name: "entryEdit", params: { pathMatch } });
}

function openRevisions() {
  router.push({ name: "revisions", params: { pathMatch } });
}

// Shared by the header Back button and the Escape-key goBack() so the two
// can't drift to different destinations.
const BACK_FALLBACK: RouteLocationRaw = { name: "entries" };

function goBack() {
  clear();
  // Pop to the page that opened this entry (normally entries). At a deep-link
  // root there's nothing to pop, so fall back to entries as the new root.
  navBack(router, BACK_FALLBACK);
}

function handleKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") {
    goBack();
  }
}
</script>

<template>
  <main class="max-w-120 mx-auto p-4" role="main" @keydown="handleKeydown">
    <BaseHeader :back-fallback="BACK_FALLBACK" @back="clear">
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

    <!-- R086 cold-start gate: a single Unlock instead of the button pile. Delete
         stays visible but de-emphasized so a delete-without-decrypt still works. -->
    <div v-if="gateVisible" class="mb-6">
      <div class="flex flex-col items-center text-muted py-6">
        <span aria-hidden="true"
          ><BaseIcon :icon="Lock" class="w-7 h-7"
        /></span>
        <p class="text-sm mt-2">{{ t("entry.unlockMicrocopy") }}</p>
      </div>
      <BaseButton
        variant="primary"
        block
        class="mb-3"
        :loading="unlocking"
        :disabled="unlocking"
        :aria-label="t('entry.unlockAria', { name: entryName })"
        @click="unlockEntry"
      >
        <BaseIcon :icon="Lock" /> {{ t("entry.unlockLabel") }}
      </BaseButton>
      <BaseButton
        variant="outline"
        block
        :disabled="unlocking || deleting"
        :aria-label="t('entry.deleteAria', { name: entryName })"
        @click="deleteSecret"
      >
        {{ deleting ? t("entry.deleting") : t("entry.deleteLabel") }}
      </BaseButton>
    </div>
    <template v-else>
      <div v-if="passwordActionsVisible" class="flex gap-3 mb-6">
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

      <!-- Attachment: metadata caption + Export. For a confirmed attachment
         Export is the primary action (Copy/Show are hidden — empty password,
         base64 body); while status is unknown Export also shows so the entry
         stays discoverable when locked. -->
      <div
        v-if="isAttachment && attachmentMeta"
        class="mb-3 text-sm text-muted flex items-center gap-1"
      >
        <BaseIcon :icon="Paperclip" />
        <span>{{
          t("entry.attachmentMeta", {
            name: attachmentMeta.filename ?? entryName,
            size: humanizeSize(attachmentMeta.size),
          })
        }}</span>
      </div>
      <BaseButton
        v-if="exportButtonVisible"
        :variant="isAttachment ? 'primary' : 'outline'"
        block
        class="mb-3"
        :disabled="loading || deleting"
        :aria-label="t('entry.exportAttachmentAria')"
        @click="exportAttachment"
      >
        <BaseIcon :icon="Download" /> {{ t("entry.exportAttachmentLabel") }}
      </BaseButton>

      <BaseButton
        v-if="totpButtonVisible"
        variant="outline"
        block
        class="mb-3"
        :disabled="loading || deleting"
        :aria-label="t('entry.copyTotpAria')"
        @click="copyTotp"
      >
        <BaseIcon :icon="Clock" /> {{ t("entry.copyTotpLabel") }}
      </BaseButton>

      <BaseButton
        variant="outline"
        block
        class="mb-3"
        :disabled="loading || deleting || editDisabled"
        :aria-label="t('entry.editAria', { name: entryName })"
        @click="editEntry"
      >
        {{ t("entry.editLabel") }}
      </BaseButton>
      <p v-if="editDisabled" class="text-center text-xs text-muted mb-3">
        {{
          editBlockedReason === "nonUtf8"
            ? t("entry.nonUtf8EditDisabledHint")
            : editBlockedReason === "legacyYaml"
              ? t("entry.legacyYamlEditDisabledHint")
              : t("entry.attachmentEditDisabledHint")
        }}
      </p>

      <BaseButton
        v-if="!isAttachment"
        variant="outline"
        block
        class="mb-3"
        :disabled="loading || deleting"
        :aria-label="t('entry.revisionsAria', { name: entryName })"
        @click="openRevisions"
      >
        {{ t("entry.revisionsLabel") }}
      </BaseButton>

      <div class="flex gap-3 mb-6">
        <BaseButton
          variant="danger"
          class="flex-1"
          :disabled="deleting || loading"
          :aria-label="t('entry.deleteAria', { name: entryName })"
          @click="deleteSecret"
        >
          {{ deleting ? t("entry.deleting") : t("entry.deleteLabel") }}
        </BaseButton>
        <BaseButton
          v-if="deleting"
          variant="outline"
          type="button"
          class="flex-1"
          :disabled="cancelling"
          :aria-label="t('entry.cancelSaveAria')"
          @click="cancelSave"
        >
          {{ cancelling ? t("entry.cancellingSave") : t("entry.cancelSave") }}
        </BaseButton>
      </div>
    </template>

    <div
      v-if="loading"
      class="flex items-center justify-center gap-2 text-center text-muted py-4"
    >
      <BaseSpinner />
      <span>{{ t("entry.decrypting") }}</span>
    </div>

    <div
      v-if="revealed && password !== null && !isAttachment"
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

      <EntryAttributes :attributes="attributes ?? []" class="mb-4" />

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

    <!-- Entry conflict modal (R026 — stale delete refused) -->
    <EntryConflictModal
      :conflict="entryConflict"
      :resolving="entryConflictResolving"
      :error="entryConflictError"
      @resolve="resolveConflict"
      @close="cancelConflict"
    />

    <!-- Full repository path, shown as quiet footer metadata -->
    <p class="text-center text-xs text-muted break-all select-all mt-8">
      {{ entryPath }}
    </p>
  </main>
</template>
