<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<script setup lang="ts">
import BaseInput from "@/components/base/BaseInput.vue";
import SshKeyFields from "@/components/SshKeyFields.vue";
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { isSshUrl as isSshRepoUrl } from "./url";

const { t } = useI18n();

// Two-way bound fields. Each consumer (RepoCloneForm, future CreateFlow)
// owns the underlying ref and passes it via v-model.
const repoUrl = defineModel<string>("repoUrl", { required: true });
const pat = defineModel<string>("pat", { required: true });
const sshKey = defineModel<string>("sshKey", { required: true });
const sshPassphrase = defineModel<string>("sshPassphrase", { required: true });
// Optional error channel so generateKey failures (inside SshKeyFields)
// surface in the parent's [role='alert'] region. Used by RepoCloneForm.
const error = defineModel<string>("error");

withDefaults(
  defineProps<{
    /** Show the Paste/Generate tab toggle + keygen UI. The Create flow will
     *  pass false for a URL+auth-only variant. */
    showKeygen?: boolean;
    /** Disable all inputs (wired to the parent's loading flag). */
    disabled?: boolean;
    /** Require a URL (native HTML5 validation). Clone needs one; Create passes
     *  false so the store can be local-only (no remote). jsdom skips HTML5
     *  validation, so the create-flow test asserts the attribute directly. */
    urlRequired?: boolean;
  }>(),
  { showKeygen: true, disabled: false, urlRequired: true },
);

// Whether the current URL is an SSH remote (delegates to the shared helper so
// clone + create classify URLs identically).
const isSshUrl = computed(() => isSshRepoUrl(repoUrl.value));
</script>

<template>
  <!-- Git Repository URL — always present -->
  <div class="flex flex-col gap-1">
    <label for="repo-url" class="text-sm font-medium">{{ t("setup.auth.repoUrlLabel") }}</label>
    <BaseInput
      id="repo-url"
      v-model="repoUrl"
      type="url"
      :placeholder="t('setup.auth.repoUrlPlaceholder')"
      :required="urlRequired"
      autocomplete="off"
      :disabled="disabled"
    />
    <small class="text-xs text-muted">{{ t("setup.auth.repoUrlHint") }}</small>
  </div>

  <!-- PAT field (shown for HTTPS URLs) -->
  <div v-if="!isSshUrl" class="flex flex-col gap-1">
    <label for="pat" class="text-sm font-medium">{{ t("setup.auth.patLabel") }}</label>
    <BaseInput
      id="pat"
      v-model="pat"
      type="password"
      :placeholder="t('setup.auth.patPlaceholder')"
      autocomplete="off"
      :disabled="disabled"
    />
    <small class="text-xs text-muted">{{ t("setup.auth.patHint") }}</small>
  </div>

  <!-- SSH key fields (shown for SSH URLs) -->
  <SshKeyFields
    v-if="isSshUrl"
    v-model:ssh-key="sshKey"
    v-model:ssh-passphrase="sshPassphrase"
    v-model:error="error"
    :show-keygen="showKeygen"
    :disabled="disabled"
  />
</template>
