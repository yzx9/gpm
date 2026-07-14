<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { addTrustedSigningKey, type AppError } from "@/api";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseHeader from "@/components/base/BaseHeader.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import BaseTextarea from "@/components/base/BaseTextarea.vue";
import { useToast } from "@/composables";
import { navBack } from "@/utils/nav";
import { Plus } from "@lucide/vue";
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter, type RouteLocationRaw } from "vue-router";

const { t } = useI18n();
const router = useRouter();
const { toast } = useToast();

// Shared by the header Back button, the form Cancel (goBack), and Save-success
// so all three return to settings and can't drift apart.
const BACK_FALLBACK: RouteLocationRaw = { name: "settings" };

const newPublicKey = ref("");
const newKeyLabel = ref("");
const saving = ref(false);
const error = ref("");

const canSave = computed(
  () => !saving.value && newPublicKey.value.trim() !== "",
);

async function onSave() {
  if (!canSave.value) return;
  saving.value = true;
  error.value = "";
  try {
    await addTrustedSigningKey(
      newPublicKey.value.trim(),
      newKeyLabel.value.trim(),
    );
    toast.success(t("addKey.addedToast"));
    navBack(router, BACK_FALLBACK);
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("addKey.addFailed");
  } finally {
    saving.value = false;
  }
}

function goBack() {
  navBack(router, BACK_FALLBACK);
}
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <BaseHeader
      :back-fallback="BACK_FALLBACK"
      :title="t('addKey.title')"
      :title-icon="Plus"
    />

    <BaseAlert v-if="error" variant="danger" class="mb-4">{{
      error
    }}</BaseAlert>

    <form class="flex flex-col gap-4" @submit.prevent="onSave">
      <div class="flex flex-col gap-1">
        <BaseTextarea
          v-model="newPublicKey"
          rows="3"
          :placeholder="t('addKey.keyPlaceholder')"
          class="font-mono text-xs"
        />
      </div>
      <div class="flex flex-col gap-1">
        <BaseInput
          v-model="newKeyLabel"
          type="text"
          :placeholder="t('addKey.labelPlaceholder')"
        />
      </div>
      <div class="flex gap-2">
        <BaseButton
          variant="action"
          class="flex-1"
          :loading="saving"
          :disabled="!canSave"
          type="submit"
        >
          {{ t("addKey.saveKey") }}
        </BaseButton>
        <BaseButton variant="secondary" class="flex-1" @click="goBack">
          {{ t("common.button.cancel") }}
        </BaseButton>
      </div>
    </form>
  </main>
</template>
