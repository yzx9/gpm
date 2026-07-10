<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { addTrustedSigningKey, type AppError } from "@/api";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import BaseTextarea from "@/components/base/BaseTextarea.vue";
import { useToast } from "@/composables";
import { navBack } from "@/utils/nav";
import { ArrowLeft, Plus } from "@lucide/vue";
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";

const { t } = useI18n();
const router = useRouter();
const { toast } = useToast();

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
    navBack(router, { name: "settings" });
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("addKey.addFailed");
  } finally {
    saving.value = false;
  }
}

function goBack() {
  navBack(router, { name: "settings" });
}
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <header class="flex justify-between items-center mb-6" role="banner">
      <h1 class="text-xl flex items-center gap-1">
        <BaseIcon :icon="Plus" :size="24" /> {{ t("addKey.title") }}
      </h1>
      <BaseButton size="sm" :aria-label="t('common.back')" @click="goBack">
        <BaseIcon :icon="ArrowLeft" /> {{ t("common.back") }}
      </BaseButton>
    </header>

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
