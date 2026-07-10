<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { listCreatePresets, type AppError, type CreatePreset } from "@/api";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseSpinner from "@/components/base/BaseSpinner.vue";
import { navBack } from "@/utils/nav";
import { ArrowLeft, Dices } from "@lucide/vue";
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

function goBack() {
  navBack(router, { name: "entries" });
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
      <h1 class="text-lg flex-1">{{ t("create.title") }}</h1>
    </header>

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
          <button
            class="type-card"
            @click="
              router.push({ name: 'createPreset', params: { presetId: p.id } })
            "
          >
            <span class="block text-base font-medium">{{ p.label }}</span>
            <span class="block text-xs text-muted"
              >{{ t("create.savedUnder") }} {{ p.prefix }}/</span
            >
          </button>
        </li>
        <li>
          <button
            class="type-card"
            @click="router.push({ name: 'createCustom' })"
          >
            <span class="block text-base font-medium">{{
              t("create.customLabel")
            }}</span>
            <span class="block text-xs text-muted">{{
              t("create.customHint")
            }}</span>
          </button>
        </li>
        <li>
          <button class="type-card" @click="openGenerate">
            <span class="flex items-center gap-2 text-base font-medium">
              <BaseIcon :icon="Dices" :size="18" />
              {{ t("create.generateLabel") }}
            </span>
            <span class="block text-xs text-muted">{{
              t("create.generateHint")
            }}</span>
          </button>
        </li>
      </ul>
    </section>
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

.type-card {
  display: block;
  width: 100%;
  text-align: left;
  padding: 0.75rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  background: var(--color-surface);
  cursor: pointer;
  min-height: 48px;
}
.type-card:active {
  background: var(--color-hover);
}
@media (hover: hover) {
  .type-card:hover {
    background: var(--color-hover);
  }
}

.loading {
  text-align: center;
  color: var(--color-muted);
  padding: 2rem 0;
}
</style>
