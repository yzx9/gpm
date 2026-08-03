<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import type { AppError, RepoConfig } from "@/api";
import { getConfig, setPat, verifyGitAuth } from "@/api";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseHeader from "@/components/base/BaseHeader.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import BaseSpinner from "@/components/base/BaseSpinner.vue";
import { useDialog, useToast } from "@/composables";
import { KeyRound, Trash2 } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

const { t } = useI18n();
const { toast } = useToast();
const { dialog } = useDialog();

const config = ref<RepoConfig | null>(null);
const loading = ref(false);
const error = ref("");

// Replace flow: stage a new token, validate it against the remote before save.
const newPat = ref("");
const verifying = ref(false);
const replacing = ref(false);
const clearing = ref(false);

const hasToken = computed(() => !!config.value?.pat);
const busy = computed(
  () => verifying.value || replacing.value || clearing.value,
);
const canReplace = computed(
  () => newPat.value.trim().length > 0 && !busy.value,
);

onMounted(loadConfig);

async function loadConfig() {
  loading.value = true;
  error.value = "";
  try {
    config.value = await getConfig();
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("pat.loadFailed");
  } finally {
    loading.value = false;
  }
}

async function replaceToken() {
  const token = newPat.value.trim();
  if (!token || busy.value) return;
  error.value = "";
  // Validate against the remote first — refuse to save a token that can't auth,
  // so the masked preview never gives false confidence in a bad/expired token.
  verifying.value = true;
  try {
    await verifyGitAuth(token);
  } catch (e) {
    verifying.value = false;
    const appError = e as AppError;
    error.value = appError?.message
      ? `${t("pat.verifyFailed")} ${appError.message}`
      : t("pat.verifyFailed");
    return;
  }
  verifying.value = false;
  replacing.value = true;
  try {
    config.value = await setPat(token);
    newPat.value = "";
    toast.success(t("pat.replaceToast"));
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("pat.replaceFailed");
  } finally {
    replacing.value = false;
  }
}

async function clearToken() {
  const confirmed = await dialog.confirm({
    message: t("pat.clearConfirm"),
    confirmLabel: t("common.button.remove"),
    danger: true,
  });
  if (!confirmed) return;
  clearing.value = true;
  error.value = "";
  try {
    config.value = await setPat(null);
    toast.success(t("pat.clearToast"));
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("pat.clearFailed");
  } finally {
    clearing.value = false;
  }
}
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <BaseHeader
      :back-fallback="{ name: 'settingsRepository' }"
      :title="t('pat.title')"
      :title-icon="KeyRound"
    />

    <BaseAlert v-if="error" variant="danger" class="mb-4">{{
      error
    }}</BaseAlert>

    <div v-if="loading" class="flex items-center gap-2 text-muted py-4">
      <BaseSpinner />
    </div>

    <template v-else>
      <p class="text-xs text-muted mb-4">{{ t("pat.description") }}</p>

      <!-- Current token (masked preview) -->
      <section class="mb-6">
        <div class="text-xs text-muted mb-1">{{ t("pat.previewLabel") }}</div>
        <code v-if="hasToken" class="key-display block">{{ config?.pat }}</code>
        <p v-else class="text-sm text-muted">{{ t("pat.noToken") }}</p>
        <p class="text-xs text-muted mt-1">{{ t("pat.maskedHelp") }}</p>
      </section>

      <!-- Replace (validated) -->
      <section class="mb-6">
        <div class="flex flex-col gap-1 mb-2">
          <label for="new-pat" class="text-xs text-muted">{{
            t("pat.replaceLabel")
          }}</label>
          <BaseInput
            id="new-pat"
            v-model="newPat"
            type="password"
            :placeholder="t('pat.replacePlaceholder')"
            autocomplete="off"
            :disabled="busy"
          />
        </div>
        <BaseButton
          variant="action"
          :loading="busy"
          :disabled="!canReplace"
          @click="replaceToken"
        >
          {{ verifying ? t("pat.verifying") : t("pat.replace") }}
        </BaseButton>
      </section>

      <!-- Clear -->
      <section v-if="hasToken">
        <BaseButton
          variant="action-danger"
          :loading="clearing"
          :disabled="busy"
          @click="clearToken"
        >
          <BaseIcon :icon="Trash2" /> {{ t("pat.clear") }}
        </BaseButton>
      </section>
    </template>
  </main>
</template>

<style scoped>
.key-display {
  padding: 0.6rem 0.75rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  background: var(--color-input);
  font-size: var(--text-xs);
  font-family: monospace;
  word-break: break-all;
}
</style>
