<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<script setup lang="ts">
import {
  editSecret,
  showPassword as showPasswordCmd,
  type AppError,
  type AttributeView,
  type DivergenceChoice,
  type EntryConflictChoice,
  type PullResult,
  type SecretParts,
} from "@/api";
import DivergenceModal from "@/components/DivergenceModal.vue";
import EntryConflictModal from "@/components/EntryConflictModal.vue";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseHeader from "@/components/base/BaseHeader.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import BaseSpinner from "@/components/base/BaseSpinner.vue";
import BaseTextarea from "@/components/base/BaseTextarea.vue";
import {
  isAuthCancelled,
  useActiveRepo,
  useCancellableSave,
  useDivergence,
  useEntryConflict,
  useLockState,
  useSecureClaim,
  useToast,
  useWipeOnLeave,
} from "@/composables";
import { currentLocale, loadBundle } from "@/i18n";
import { navBack } from "@/utils/nav";
import { Eye, EyeOff, Trash2 } from "@lucide/vue";
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
const activeRepo = useActiveRepo();
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
// The secret's attributes, edited as structured rows (R069 2b). Round-tripped
// unchanged until the row UI lands; the editor never mirrors `to_bytes` in TS —
// Rust reassembles the on-disk plaintext from these parts.
const editAttributes = ref<AttributeView[]>([]);
// The structured snapshot captured at load, for the no-op-save dirty-check.
const loadedParts = ref<SecretParts>({
  password: "",
  attributes: [],
  body: "",
});
// R026: the blob oid captured at load (the base version) — sent back on save so
// a stale edit surfaces entry_conflict instead of silently clobbering a teammate.
const baseOid = ref<string | null>(null);
const loading = ref(false);
const saving = ref(false);
const error = ref("");
// Caught backend error code, so the save-failure alert can render as a warning
// (known platform limitation) for PLUGIN_UNAVAILABLE instead of the default red
// "danger". Null outside the catch; reset wherever `error` is.
const errorCode = ref<string | null>(null);
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

const {
  conflict: entryConflict,
  resolving: entryConflictResolving,
  conflictError: entryConflictError,
  openConflict,
  resolveConflict,
  cancelConflict,
} = useEntryConflict({
  resolveFailedKey: "entry.resolveFailed",
  onResolved(result: PullResult, choice: EntryConflictChoice) {
    exitEdit();
    toast.success(
      choice === "keep_mine"
        ? t("entry.saved", { commit: result.head })
        : t("entry.revertedToTheirs"),
    );
    navBack(router, { name: "entries" });
  },
  onPullFfFailed() {
    toast.info(t("entry.remoteChanged"));
    navBack(router, { name: "entries" });
  },
  onAuthenticityBlocked() {
    // Enforce refused the resolve's re-fetch — nothing saved. Stay + explain
    // (mirrors the save path's authenticity_blocked branch).
    error.value = t("entry.saveBlocked");
  },
});

onMounted(loadBody);

/** The structured parts the editor currently holds — what `onSave` sends and the
 *  dirty-check compares against `loadedParts`. */
const currentParts = computed<SecretParts>(() => ({
  password: editPassword.value,
  attributes: editAttributes.value,
  body: editNotes.value,
}));

/** Deep snapshot of the current parts (independent of the live refs) — the
 *  dirty-check baseline captured at load. */
function snapshotParts(): SecretParts {
  return {
    password: editPassword.value,
    attributes: editAttributes.value.map((a) => ({
      key: a.key,
      value: a.value,
    })),
    body: editNotes.value,
  };
}

/** Structural equality of two parts snapshots (deep on the attribute array). */
function partsEqual(a: SecretParts, b: SecretParts): boolean {
  if (a.password !== b.password || a.body !== b.body) return false;
  if (a.attributes.length !== b.attributes.length) return false;
  return a.attributes.every(
    (attr, i) =>
      attr.key === b.attributes[i].key && attr.value === b.attributes[i].value,
  );
}

/** A key whose value is secret-bearing — masked in the editor (mirrors
 *  EntryAttributes' display heuristic). */
function isSensitiveKey(key: string): boolean {
  return /password|secret|pin|token|passphrase|api[_-]?key|access[_-]?key|bearer|credential/i.test(
    key,
  );
}

/** Per-row reveal state for sensitive attribute values (masked by default). */
const revealedAttr = ref<Record<number, boolean>>({});

/** Append a blank attribute row. */
function addAttribute() {
  editAttributes.value.push({ key: "", value: "" });
}

/** Remove the attribute row at `index`. */
function removeAttribute(index: number) {
  editAttributes.value.splice(index, 1);
}

/** Any attribute key carrying the gopass `": "` separator or a newline would
 *  re-parse to a different structure — block Save (the Rust assembler also
 *  rejects it via SecretInvalid). */
const hasInvalidKey = computed(() =>
  editAttributes.value.some(
    (a) => a.key.includes(": ") || a.key.includes("\n"),
  ),
);

/** A004: gpm never writes YAML secrets — parts that would assemble to a
 *  `---`-marked body land the entry read-only on its very next read (the
 *  markdown horizontal-rule paste). Mirrors the backend's
 *  `is_yaml_secret_content` rule (Store::set refuses it too, this is the
 *  earlier inline hint): the password counts only as a bare `---` document;
 *  later lines are markers when they start `---` but not `----` (PEM armor
 *  stays editable, matching gopass's effective classification). */
const hasYamlMarker = computed(() => {
  const bareDoc = editPassword.value.trim() === "---";
  const attrLines = editAttributes.value.map((a) => `${a.key}: ${a.value}`);
  const bodyLines = editNotes.value.split("\n");
  const markerLine = [...attrLines, ...bodyLines].some(
    (line) => line.startsWith("---") && !line.startsWith("----"),
  );
  return bareDoc || markerLine;
});

/** The editor's dirty predicate — has effectively-non-empty content AND differs
 *  from the loaded baseline. age ciphertext is non-deterministic, so an
 *  unchanged Save would still make a spurious commit (block it); an
 *  effectively-empty secret would be rejected by `Secret::parse` on the next
 *  read, bricking it (block it). The trim is on the GATE only — the saved parts
 *  stay untrimmed (lossless). Shared by the Save gate (`canSave`) and the lock
 *  path's drafts-notice mark (`exitEdit`), so the two can never diverge. */
function hasUnsavedParts(): boolean {
  const p = currentParts.value;
  const hasContent =
    p.password.trim() !== "" ||
    p.body.trim() !== "" ||
    p.attributes.some((a) => a.key.trim() !== "" || a.value.trim() !== "");
  return hasContent && !partsEqual(p, loadedParts.value);
}

/** Save is enabled only when the editor holds unsaved (dirty) parts. */
const canSave = computed(() => {
  if (
    saving.value ||
    isAttachment.value ||
    isNonUtf8.value ||
    hasInvalidKey.value ||
    hasYamlMarker.value
  )
    return false;
  return hasUnsavedParts();
});

async function loadBody() {
  loading.value = true;
  error.value = "";
  decryptError.value = false;
  const myToken = ++loadToken;
  try {
    const repoId = await activeRepo.currentId();
    // withClaim raises FLAG_SECURE before the decrypted body arrives; a late
    // load resolving after we left is discarded by the token; a failed acquire
    // returns null → abort (the per-op replacement for the old route-guard abort).
    const claimed = await withClaim(() =>
      runWithAuth(() => showPasswordCmd(repoId, entryPath)),
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
    if (claimed.edit_blocked === "legacyYaml") {
      // A legacy gopass YAML secret is read-only (A004): gpm would corrupt it
      // on the canonical write-back. Block at the source — covers the detail
      // page's pre-probe window and direct /edit deep-links alike.
      isAttachment.value = true; // reuse the same "blocked editor" affordance
      error.value = t("entry.legacyYamlEditDisabledHint");
      return;
    }
    editPassword.value = claimed.password ?? "";
    editNotes.value = claimed.notes ?? "";
    editAttributes.value = (claimed.attributes ?? []).map((a) => ({
      key: a.key,
      value: a.value,
    }));
    loadedParts.value = snapshotParts();
    baseOid.value = claimed.version ?? null;
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

/** Returns whether the editor held unsaved edits (`hasUnsavedParts` — merely
 *  opening an entry clears loaded plaintext but loses no user content, so the
 *  lock path only marks the drafts notice when this is true). */
function exitEdit(): boolean {
  const hadEdits = hasUnsavedParts();
  editPassword.value = "";
  editNotes.value = "";
  editAttributes.value = [];
  loadedParts.value = { password: "", attributes: [], body: "" };
  baseOid.value = null;
  return hadEdits;
}

// Wipe the working plaintext on browser back, unmount, and either lock (a
// gate re-lock's mask covers this page without unmounting it) so it doesn't
// survive behind a wiped identity. (useDivergence clears its own modal state
// on lock.)
useWipeOnLeave(exitEdit);

async function onSave() {
  if (!canSave.value) return;
  saving.value = true;
  error.value = "";
  errorCode.value = null;
  decryptError.value = false;
  try {
    const parts = snapshotParts();
    const repoId = await activeRepo.currentId();
    const outcome = await editSecret(repoId, entryName, parts, baseOid.value);
    if (outcome.kind === "written") {
      toast.success(t("entry.saved", { commit: outcome.commit }));
      // Back to the read view (the opener) — it remounts and shows fresh content.
      navBack(router, BACK_FALLBACK);
    } else if (outcome.kind === "entry_conflict") {
      // R026: the entry changed on the remote since the read — refuse the stale
      // edit and let the user pick. Stay on the form; the plaintext is preserved.
      const { kind: _kind, ...payload } = outcome;
      void _kind;
      openConflict(payload, parts);
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
    errorCode.value = appError?.code ?? null;
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

    <BaseAlert
      v-if="error"
      :variant="errorCode === 'PLUGIN_UNAVAILABLE' ? 'warning' : 'danger'"
      class="mb-4"
    >
      {{ error }}
      <span v-if="decryptError" class="block text-xs opacity-80 mt-1">
        {{ t("entry.checkIdentityHint") }}
      </span>
    </BaseAlert>

    <BaseAlert v-if="hasYamlMarker" variant="warning" class="mb-4">
      {{ t("entry.yamlMarkerBlocked") }}
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
      <div class="flex flex-col gap-2">
        <div
          v-for="(attr, index) in editAttributes"
          :key="index"
          class="flex flex-col gap-1"
        >
          <div class="flex items-center gap-2">
            <BaseInput
              v-model="attr.key"
              class="flex-1"
              autocomplete="off"
              autocapitalize="off"
              autocorrect="off"
              spellcheck="false"
              :placeholder="t('entry.attrKeyAria')"
              :aria-label="t('entry.attrKeyAria')"
            />
            <BaseButton
              variant="link"
              tone="muted"
              type="button"
              :aria-label="t('entry.removeAttrAria')"
              @click="removeAttribute(index)"
            >
              <BaseIcon :icon="Trash2" />
            </BaseButton>
          </div>
          <div class="flex items-center gap-2">
            <BaseInput
              v-model="attr.value"
              class="flex-1 font-mono"
              :type="
                isSensitiveKey(attr.key) && !revealedAttr[index]
                  ? 'password'
                  : 'text'
              "
              autocomplete="off"
              autocapitalize="off"
              autocorrect="off"
              spellcheck="false"
              :placeholder="t('entry.attrValueAria')"
              :aria-label="t('entry.attrValueAria')"
            />
            <BaseButton
              v-if="isSensitiveKey(attr.key)"
              variant="link"
              tone="muted"
              type="button"
              :aria-label="
                revealedAttr[index]
                  ? t('entry.hideValueAria')
                  : t('entry.showValueAria')
              "
              @click="revealedAttr[index] = !revealedAttr[index]"
            >
              <BaseIcon :icon="revealedAttr[index] ? EyeOff : Eye" />
            </BaseButton>
          </div>
          <p
            v-if="attr.key.includes(': ') || attr.key.includes('\n')"
            class="text-xs text-danger"
          >
            {{ t("entry.attrKeyInvalid") }}
          </p>
        </div>
        <BaseButton
          variant="link"
          tone="muted"
          type="button"
          class="self-start"
          @click="addAttribute"
        >
          + {{ t("entry.addAttribute") }}
        </BaseButton>
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

    <!-- Entry conflict modal (R026 — stale edit refused) -->
    <EntryConflictModal
      :conflict="entryConflict"
      :resolving="entryConflictResolving"
      :error="entryConflictError"
      @resolve="resolveConflict"
      @close="cancelConflict"
    />
  </main>
</template>
