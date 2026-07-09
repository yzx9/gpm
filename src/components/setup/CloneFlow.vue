<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { isRepoReady } from "@/api";
import { useWipeOnLeave } from "@/composables";
import { computed, onMounted, ref } from "vue";
import IdentitySetupForm from "./IdentitySetupForm.vue";
import RepoCloneForm from "./RepoCloneForm.vue";
import { isSshUrl as isSshRepoUrl } from "./url";

const emit = defineEmits<{
  done: [];
}>();

// Step indicator (1 = clone, 2 = identity).
const step = ref(1);

// Step-1 auth fields hoisted here so they survive the 1↔2 transition —
// IdentitySetupForm's "Use my SSH key for decryption" reads `sshKey`.
const repoUrl = ref("");
const pat = ref("");
const sshKey = ref("");
const sshPassphrase = ref("");

const isSshUrl = computed(() => isSshRepoUrl(repoUrl.value));

// Wipe the hoisted git credentials on browser back and when CloneFlow unmounts.
// CloneFlow owns `step` and only swaps its v-if children, so neither trigger fires
// on the internal 1↔2 step change — the "survive the round-trip" UX is preserved.
// (Browser-back during setup is a leave: popstate fires, then the flow unmounts.)
// No lock wiring during setup.
useWipeOnLeave(
  () => {
    pat.value = "";
    sshKey.value = "";
    sshPassphrase.value = "";
  },
  { lock: false },
);

// Auto-advance to step 2 if repo is already cloned (identity missing).
onMounted(async () => {
  try {
    const ready = await isRepoReady();
    if (ready) {
      step.value = 2;
    }
  } catch {
    // Not ready — stay on step 1
  }
});

function onCloneDone() {
  step.value = 2;
}
function onIdentityBack() {
  step.value = 1;
}
function onIdentityDone() {
  emit("done");
}
</script>

<template>
  <!-- Step indicator -->
  <div class="flex items-center justify-center gap-2 mb-6">
    <span
      :class="[
        'inline-flex items-center justify-center w-7 h-7 rounded-full text-xs font-bold',
        step >= 1 ? 'bg-accent text-on-accent' : 'bg-edge text-muted',
      ]"
      >1</span
    >
    <div :class="['h-0.5 w-8', step >= 2 ? 'bg-accent' : 'bg-edge']"></div>
    <span
      :class="[
        'inline-flex items-center justify-center w-7 h-7 rounded-full text-xs font-bold',
        step >= 2 ? 'bg-accent text-on-accent' : 'bg-edge text-muted',
      ]"
      >2</span
    >
  </div>

  <RepoCloneForm
    v-if="step === 1"
    v-model:repo-url="repoUrl"
    v-model:pat="pat"
    v-model:ssh-key="sshKey"
    v-model:ssh-passphrase="sshPassphrase"
    @done="onCloneDone"
  />
  <IdentitySetupForm
    v-else
    :ssh-key="sshKey"
    :is-ssh-url="isSshUrl"
    @back="onIdentityBack"
    @done="onIdentityDone"
  />
</template>
