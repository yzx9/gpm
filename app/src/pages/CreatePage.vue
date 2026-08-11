<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<script setup lang="ts">
import { listCreatePresets, type AppError, type CreatePreset } from "@/api";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseHeader from "@/components/base/BaseHeader.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseSpinner from "@/components/base/BaseSpinner.vue";
import { Dices } from "@lucide/vue";
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";

const { t } = useI18n();
const router = useRouter();

const presets = ref<CreatePreset[]>([]);
const presetsLoading = ref(true);
const error = ref("");

async function loadPresets() {
  presetsLoading.value = true;
  try {
    presets.value = await listCreatePresets();
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("create.presetsFailed");
    console.warn("[create] presets load failed", e);
  } finally {
    presetsLoading.value = false;
  }
}

/** The generate card routes to the standalone generator (which only copies to
 *  the clipboard — it saves nothing). Kept inside the ＋ flow because "generate
 *  a one-off password" is the same intent as "create a secret", just without
 *  persistence. */
function openGenerate() {
  router.push({ name: "generate" });
}

onMounted(loadPresets);
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <BaseHeader :back-fallback="{ name: 'entries' }">
      <template #title>
        <h1 class="text-lg flex-1">{{ t("create.title") }}</h1>
      </template>
    </BaseHeader>

    <BaseAlert v-if="error" variant="danger" class="mb-3">{{
      error
    }}</BaseAlert>

    <!-- Pick a type — each routes to its own page so Android back returns here -->
    <section>
      <p class="text-sm text-muted mb-3">{{ t("create.pickHint") }}</p>
      <div v-if="presetsLoading" class="loading">
        <BaseSpinner /> {{ t("create.loading") }}
      </div>
      <ul v-else class="list-none flex flex-col gap-2" role="list">
        <li v-for="p in presets" :key="p.id">
          <BaseButton
            variant="action"
            @click="
              router.push({ name: 'createPreset', params: { presetId: p.id } })
            "
          >
            <span class="flex flex-col flex-1">
              <span class="text-base font-medium">{{ p.label }}</span>
              <span class="text-xs text-muted"
                >{{ t("create.savedUnder") }} {{ p.prefix }}/</span
              >
            </span>
          </BaseButton>
        </li>
        <li>
          <BaseButton
            variant="action"
            @click="router.push({ name: 'createCustom' })"
          >
            <span class="flex flex-col flex-1">
              <span class="text-base font-medium">{{
                t("create.customLabel")
              }}</span>
              <span class="text-xs text-muted">{{
                t("create.customHint")
              }}</span>
            </span>
          </BaseButton>
        </li>
        <li>
          <BaseButton variant="action" @click="openGenerate">
            <span class="flex flex-col flex-1">
              <span class="flex items-center gap-2 text-base font-medium">
                <BaseIcon :icon="Dices" :size="18" />
                {{ t("create.generateLabel") }}
              </span>
              <span class="text-xs text-muted">{{
                t("create.generateHint")
              }}</span>
            </span>
          </BaseButton>
        </li>
      </ul>
    </section>
  </main>
</template>

<style scoped>
.loading {
  text-align: center;
  color: var(--color-muted);
  padding: 2rem 0;
}
</style>
