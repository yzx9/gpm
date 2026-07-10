<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import {
  createSecret,
  lookupTemplate,
  previewCreate,
  type AppError,
  type DivergenceChoice,
  type PullResult,
} from "@/api";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseInput from "@/components/base/BaseInput.vue";
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
import { computed, onBeforeUnmount, ref, watch } from "vue";
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
  } catch {
    // Invalid name mid-typing, or a template references an unknown var — no preview.
    hasTemplate.value = false;
    preview.value = null;
  }
}

const canSubmit = computed(
  () =>
    !submitting.value &&
    customName.value.trim() !== "" &&
    customContent.value.trim() !== "",
);

async function onSave() {
  if (!canSubmit.value) return;
  submitting.value = true;
  error.value = "";
  try {
    const outcome = await runWithAuth(() =>
      createSecret(customName.value.trim(), customContent.value),
    );
    if (outcome.kind === "written") {
      toast.success(t("create.saved", { commit: outcome.commit }));
      navBack(router, { name: "entries" });
    } else if (outcome.kind === "needs_divergence_resolve") {
      const { kind: _kind, ...preview } = outcome;
      void _kind;
      openDivergence(preview);
    } else {
      error.value = t("create.saveBlocked");
    }
  } catch (e) {
    if (isAuthCancelled(e)) return;
    const appError = e as AppError;
    error.value = appError?.message || t("create.createFailed");
  } finally {
    submitting.value = false;
  }
}

// The unlock modal keeps this page mounted on auto-lock, so wipe any half-typed
// secret the moment the identity locks.
function wipeCustom() {
  customName.value = "";
  customContent.value = "";
}
useWipeOnLeave(wipeCustom);

onBeforeUnmount(() => {
  if (previewTimer) clearTimeout(previewTimer);
});

function goBack() {
  navBack(router, { name: "create" });
}
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <header class="flex items-center gap-3 mb-6" role="banner">
      <button
        @click="goBack"
        class="back-btn inline-flex items-center gap-1"
        :aria-label="t('common.back')"
      >
        <BaseIcon :icon="ArrowLeft" /> {{ t("common.back") }}
      </button>
      <h1 class="text-lg flex-1">{{ t("create.customLabel") }}</h1>
    </header>

    <BaseAlert v-if="error" variant="danger" class="mb-3">{{
      error
    }}</BaseAlert>

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
      <pre v-if="preview" class="preview">{{ preview }}</pre>
      <BaseButton variant="primary" type="submit" :disabled="!canSubmit">{{
        submitting ? t("create.saving") : t("create.saveSecret")
      }}</BaseButton>
    </form>

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

<style scoped>
.back-btn {
  background: transparent;
  border: none;
  font-size: var(--text-base);
  cursor: pointer;
  color: var(--color-accent);
  padding: 0.25rem 0.5rem;
  min-width: 48px;
  min-height: 48px;
}

.preview {
  padding: 0.5rem 0.75rem;
  background: var(--color-accent-ring);
  border-radius: var(--radius-sm);
  font-size: var(--text-sm);
  white-space: pre-wrap;
  break-all: word-break;
}
</style>
