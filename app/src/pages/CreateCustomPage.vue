<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<script setup lang="ts">
import {
  createSecret,
  lookupTemplate,
  previewCreate,
  type AppError,
  type DivergenceChoice,
  type EntryConflictChoice,
  type PullResult,
  type SecretParts,
} from "@/api";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseHeader from "@/components/base/BaseHeader.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import BaseTextarea from "@/components/base/BaseTextarea.vue";
import DivergenceModal from "@/components/DivergenceModal.vue";
import EntryConflictModal from "@/components/EntryConflictModal.vue";
import {
  isAuthCancelled,
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
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";

// Reuse the `create.*` bundle (loaded for the pick step); load explicitly for a
// cold deep-link to /create/custom without a prior /create visit.
void loadBundle(currentLocale(), "create");

const { t } = useI18n();
const router = useRouter();
const { runWithAuth } = useLockState();
const { toast } = useToast();

const customName = ref("");
const customContent = ref("");
const submitting = ref(false);
const error = ref("");
// Caught backend error code — renders the alert as a warning (not red danger)
// for PLUGIN_UNAVAILABLE (a known platform limitation, not a transient failure).
const errorCode = ref<string | null>(null);
const { cancelling, cancelSave } = useCancellableSave();

/** Split raw custom-create content into structured parts for a keep-mine conflict
 *  resolve: first line is the password, the rest is the body, no attributes.
 *  `Secret::from_parts` → `to_bytes` reassembles byte-identically (any trailing
 *  newline normalizes on the next read). */
function contentToParts(content: string): SecretParts {
  const nl = content.indexOf("\n");
  return nl === -1
    ? { password: content, attributes: [], body: "" }
    : {
        password: content.slice(0, nl),
        attributes: [],
        body: content.slice(nl + 1),
      };
}

// Template hint / live preview (location-based, gopass).
const hasTemplate = ref(false);
const preview = ref<string | null>(null);
let previewTimer: ReturnType<typeof setTimeout> | null = null;

const {
  divergence,
  resolving,
  divergeError,
  openDivergence,
  resolveDivergence,
  cancelDivergence,
} = useDivergence({
  resolveFailedKey: "create.resolveFailed",
  onResolved(result: PullResult, choice: DivergenceChoice) {
    if (choice === "adopt_remote") {
      toast.info(t("create.adoptedRemote"));
    } else {
      toast.success(t("create.keptMine", { head: result.head }));
    }
    navBack(router, { name: "entries" });
  },
  onPullFfFailed() {
    toast.info(t("create.remoteChanged"));
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
  resolveFailedKey: "create.resolveFailed",
  onResolved(result: PullResult, choice: EntryConflictChoice) {
    toast.success(
      choice === "keep_mine"
        ? t("create.saved", { commit: result.head })
        : t("create.keptTheirs"),
    );
    navBack(router, { name: "entries" });
  },
  onPullFfFailed() {
    toast.info(t("create.remoteChanged"));
    navBack(router, { name: "entries" });
  },
  onAuthenticityBlocked() {
    // Enforce refused the resolve's re-fetch — nothing created. Stay + explain
    // (mirrors the save path's authenticity_blocked branch).
    error.value = t("create.saveBlocked");
  },
});

// Debounced template lookup + preview (location-based, gopass).
watch([customName, customContent], () => {
  if (previewTimer) clearTimeout(previewTimer);
  previewTimer = setTimeout(refreshPreview, 200);
});

async function refreshPreview() {
  const name = customName.value.trim();
  if (name === "") {
    hasTemplate.value = false;
    preview.value = null;
    return;
  }
  try {
    hasTemplate.value = (await lookupTemplate(name)) !== null;
    preview.value = await previewCreate(name, customContent.value);
  } catch (e) {
    // Invalid name mid-typing, or a template references an unknown var — no preview.
    console.debug("[create-custom] preview failed", e);
    hasTemplate.value = false;
    preview.value = null;
  }
}

/** A004: gpm never creates YAML secrets — content the read path would
 *  classify as legacy YAML lands read-only on its very next read. Mirrors the
 *  backend's `is_yaml_secret_content`: the first line counts only as a bare
 *  `---` document (the password line is never a marker — gopass consumes it
 *  before peeking), a marker line after it starts `---` but NOT `----` (PEM
 *  armor is an editable AKV body in gopass's effective classification), so
 *  the form blocks before the round-trip error. */
const hasYamlMarker = computed(() => {
  const lines = customContent.value.split("\n");
  const bareDoc = (lines[0] ?? "").trim() === "---";
  const markerLine = lines
    .slice(1)
    .some((line) => line.startsWith("---") && !line.startsWith("----"));
  return bareDoc || markerLine;
});

const canSubmit = computed(
  () =>
    !submitting.value &&
    !hasYamlMarker.value &&
    customName.value.trim() !== "" &&
    customContent.value.trim() !== "",
);

async function onSave() {
  if (!canSubmit.value) return;
  submitting.value = true;
  error.value = "";
  errorCode.value = null;
  try {
    const outcome = await runWithAuth(() =>
      createSecret(customName.value.trim(), customContent.value),
    );
    if (outcome.kind === "written") {
      toast.success(t("create.saved", { commit: outcome.commit }));
      navBack(router, { name: "entries" });
    } else if (outcome.kind === "entry_conflict") {
      // R026: a teammate created the same name — refuse and let the user pick.
      const { kind: _kind, ...payload } = outcome;
      void _kind;
      openConflict(payload, contentToParts(customContent.value));
    } else if (outcome.kind === "needs_divergence_resolve") {
      const { kind: _kind, ...preview } = outcome;
      void _kind;
      openDivergence(preview);
    } else if (outcome.kind === "cancelled") {
      // User aborted. Nothing was published; if committed, it stays local and
      // syncs next time. Stay on the form — neutral status, not an error.
      toast.info(
        outcome.committed
          ? t("create.saveCancelledLocalStays")
          : t("create.saveCancelledNothingPublished"),
      );
    } else {
      error.value = t("create.saveBlocked");
    }
  } catch (e) {
    if (isAuthCancelled(e)) return;
    const appError = e as AppError;
    errorCode.value = appError?.code ?? null;
    error.value = appError?.message || t("create.createFailed");
    console.warn("[create-custom] create failed", e);
  } finally {
    submitting.value = false;
    cancelling.value = false;
  }
}

// The unlock modal keeps this page mounted on auto-lock, so wipe any half-typed
// secret the moment the identity locks.
function wipeCustom() {
  customName.value = "";
  customContent.value = "";
}
useWipeOnLeave(wipeCustom);

// R031: this form authors a secret, so hold a screen-capture claim for the
// page's lifetime (released at unmount via onScopeDispose).
const { acquire: acquireSecure } = useSecureClaim();
onMounted(() => {
  void acquireSecure();
});

onBeforeUnmount(() => {
  if (previewTimer) clearTimeout(previewTimer);
});
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <BaseHeader :back-fallback="{ name: 'create' }">
      <template #title>
        <h1 class="text-lg flex-1">{{ t("create.customLabel") }}</h1>
      </template>
    </BaseHeader>

    <BaseAlert
      v-if="error"
      :variant="errorCode === 'PLUGIN_UNAVAILABLE' ? 'warning' : 'danger'"
      class="mb-3"
    >
      {{ error }}
    </BaseAlert>

    <form class="flex flex-col gap-4" @submit.prevent="onSave">
      <div class="flex flex-col gap-1">
        <label for="c-name" class="text-sm font-medium">
          {{ t("create.pathName") }}<span aria-hidden="true">*</span>
        </label>
        <BaseInput
          id="c-name"
          v-model="customName"
          type="text"
          :placeholder="t('create.pathPlaceholder')"
          autocomplete="off"
        />
        <small class="text-xs text-muted">{{
          t("create.firstLineHint")
        }}</small>
      </div>
      <div class="flex flex-col gap-1">
        <label for="c-content" class="text-sm font-medium">
          {{ t("create.content") }}<span aria-hidden="true">*</span>
        </label>
        <BaseTextarea
          id="c-content"
          v-model="customContent"
          rows="4"
          autocomplete="off"
        />
      </div>
      <BaseAlert v-if="hasTemplate" variant="info">
        {{ t("create.templateHint") }}
      </BaseAlert>
      <BaseAlert v-if="hasYamlMarker" variant="warning">
        {{ t("create.yamlMarkerBlocked") }}
      </BaseAlert>
      <pre v-if="preview" class="preview">{{ preview }}</pre>
      <div class="flex gap-3">
        <BaseButton
          variant="primary"
          type="submit"
          class="flex-1"
          :disabled="!canSubmit"
          >{{ submitting ? t("create.saving") : t("create.saveSecret") }}
        </BaseButton>
        <BaseButton
          v-if="submitting"
          variant="outline"
          type="button"
          class="flex-1"
          :disabled="cancelling"
          :aria-label="t('create.cancelSaveAria')"
          @click="cancelSave"
          >{{
            cancelling ? t("create.cancellingSave") : t("create.cancelSave")
          }}
        </BaseButton>
      </div>
    </form>

    <DivergenceModal
      context="save"
      :divergence="divergence"
      :resolving="resolving"
      :error="divergeError"
      @resolve="resolveDivergence"
      @close="cancelDivergence"
    />

    <EntryConflictModal
      :conflict="entryConflict"
      :resolving="entryConflictResolving"
      :error="entryConflictError"
      @resolve="resolveConflict"
      @close="cancelConflict"
    />
  </main>
</template>

<style scoped>
.preview {
  padding: 0.5rem 0.75rem;
  background: var(--color-accent-ring);
  border-radius: var(--radius-sm);
  font-size: var(--text-sm);
  white-space: pre-wrap;
  break-all: word-break;
}
</style>
