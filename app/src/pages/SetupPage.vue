<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<script setup lang="ts">
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseSelect from "@/components/base/BaseSelect.vue";
import CloneFlow from "@/components/setup/CloneFlow.vue";
import CreateFlow from "@/components/setup/CreateFlow.vue";
import CreateGpgFlow from "@/components/setup/CreateGpgFlow.vue";
import { useSecureClaim } from "@/composables";
import { LockKeyhole } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";

const router = useRouter();
const { t } = useI18n();

// Mode switch. Defaults to "clone" so CloneFlow mounts immediately on
// first render — this preserves the existing SetupPage test contract, which
// mounts SetupPage and expects the clone flow to be live without any click.
const mode = ref<"clone" | "create" | "createGpg">("clone");

// Typed so BaseSelect's T resolves to the mode union (not `string`): the
// @change handler then takes the union directly, with no `as` cast that could
// silently admit a drifted option value.
const modeOptions = computed<
  {
    label: string;
    value: "clone" | "create" | "createGpg";
  }[]
>(() => [
  { label: t("setup.mode.clone"), value: "clone" },
  { label: t("setup.mode.create"), value: "create" },
  { label: t("setup.mode.createGpg"), value: "createGpg" },
]);

function onModeChange(next: "clone" | "create" | "createGpg") {
  mode.value = next;
}

// R031: setup collects git credentials + an identity (CloneFlow/CreateFlow
// children), so hold a screen-capture claim for the route's lifetime. FLAG_SECURE
// is window-level, so this one claim covers every hosted input across both flows.
const { acquire: acquireSecure } = useSecureClaim();
onMounted(() => {
  void acquireSecure();
});

function onDone() {
  // Setup is terminal — replace so Back can't return to the setup flow.
  router.replace({ name: "entries" });
}
</script>

<template>
  <main
    class="min-h-screen flex items-center justify-center max-[480px]:items-start p-4 max-[480px]:pt-6 max-[480px]:pb-0"
    role="main"
  >
    <div
      class="w-full max-w-105 bg-surface rounded-lg p-8 shadow-[0_2px_12px_rgba(0,0,0,0.08)] max-[480px]:p-4 max-[480px]:pb-28"
    >
      <h1
        class="text-center text-display mb-1 flex items-center justify-center gap-2"
      >
        <BaseIcon :icon="LockKeyhole" :size="28" /> gpm
      </h1>
      <p class="text-center text-muted text-sm mb-6">
        {{ t("setup.tagline") }}
      </p>

      <div class="mb-6">
        <BaseSelect
          name="setup-mode"
          :legend="t('setup.mode.label')"
          :model-value="mode"
          :options="modeOptions"
          @change="onModeChange"
        />
      </div>

      <CloneFlow v-if="mode === 'clone'" @done="onDone" />
      <CreateGpgFlow v-else-if="mode === 'createGpg'" @done="onDone" />
      <CreateFlow v-else @done="onDone" />
    </div>
  </main>
</template>
