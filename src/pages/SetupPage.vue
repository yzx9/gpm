<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import BaseIcon from "@/components/base/BaseIcon.vue";
import CloneFlow from "@/components/setup/CloneFlow.vue";
import CreateFlow from "@/components/setup/CreateFlow.vue";
import { LockKeyhole } from "@lucide/vue";
import { ref } from "vue";
import { useRouter } from "vue-router";

const router = useRouter();

// Mode switch. Defaults to "clone" so CloneFlow mounts immediately on
// first render — this preserves the existing SetupPage test contract, which
// mounts SetupPage and expects the clone flow to be live without any click.
// Rendered as a <select> (not buttons) so it does not pollute
// `findAll("button[type='button'])` — see the back-button ordering test.
const mode = ref<"clone" | "create">("clone");

function onDone() {
  router.push({ name: "entries" });
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
        Age-only gopass password client
      </p>

      <!-- Mode switch (a <select>, not buttons — see script comment). -->
      <div class="flex flex-col gap-1 mb-6">
        <label for="setup-mode" class="text-sm font-medium">Mode</label>
        <select
          id="setup-mode"
          v-model="mode"
          class="input-base"
          autocomplete="off"
        >
          <option value="clone">Clone an existing store</option>
          <option value="create">Create a new store</option>
        </select>
      </div>

      <CloneFlow v-if="mode === 'clone'" @done="onDone" />
      <CreateFlow v-else @done="onDone" />
    </div>
  </main>
</template>

<style scoped>
.input-base {
  padding: 0.6rem 0.75rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  font-size: var(--text-base);
  font-family: inherit;
  background: var(--color-input);
  color: inherit;
  min-height: 48px;
}

.input-base:focus {
  outline: none;
  border-color: var(--color-accent);
  box-shadow: 0 0 0 2px var(--color-accent-ring);
}
</style>
