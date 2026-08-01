<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import type { AppError } from "@/api";
import { generateSshKey } from "@/api";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import BaseTextarea from "@/components/base/BaseTextarea.vue";
import PassphraseField from "@/components/PassphraseField.vue";
import { CircleCheck, Copy, KeyRound } from "@lucide/vue";
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { isSshUrl as isSshRepoUrl } from "./url";

const { t } = useI18n();

// Two-way bound fields. Each consumer (RepoCloneForm, future CreateFlow)
// owns the underlying ref and passes it via v-model.
const repoUrl = defineModel<string>("repoUrl", { required: true });
const pat = defineModel<string>("pat", { required: true });
const sshKey = defineModel<string>("sshKey", { required: true });
const sshPassphrase = defineModel<string>("sshPassphrase", { required: true });
// Optional error channel so generateKey failures surface in the parent's
// [role='alert'] region. Used by RepoCloneForm.
const error = defineModel<string>("error");

withDefaults(
  defineProps<{
    /** Show the Paste/Generate tab toggle + keygen UI. The Create flow will
     *  pass false for a URL+auth-only variant. */
    showKeygen?: boolean;
    /** Disable all inputs (wired to the parent's loading flag). */
    disabled?: boolean;
  }>(),
  { showKeygen: true, disabled: false },
);

// Whether the current URL is an SSH remote (delegates to the shared helper so
// clone + create classify URLs identically).
const isSshUrl = computed(() => isSshRepoUrl(repoUrl.value));

// SSH key generation state — owned here, same behavior as the original.
const sshKeySource = ref<"paste" | "generate">("paste");
const generatedPublicKey = ref("");
const generating = ref(false);
// Confirm-field controller for the generated-key passphrase (validate/reset).
const pf = ref<InstanceType<typeof PassphraseField> | null>(null);

async function generateKey() {
  error.value = "";
  // The passphrase encrypts the freshly generated key — a typo can't be
  // recovered, so require a matching confirm before generating.
  const passphraseError = pf.value?.validate() ?? null;
  if (passphraseError) {
    error.value = passphraseError;
    return;
  }
  generating.value = true;
  try {
    const result = await generateSshKey(sshPassphrase.value || null);
    sshKey.value = result.private_key;
    generatedPublicKey.value = result.public_key;
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("setup.auth.errKeyGen");
  } finally {
    generating.value = false;
  }
}

async function copyPublicKey() {
  try {
    await navigator.clipboard.writeText(generatedPublicKey.value);
  } catch {
    // Fallback — select text for manual copy
  }
}
</script>

<template>
  <!-- Git Repository URL — always present -->
  <div class="flex flex-col gap-1">
    <label for="repo-url" class="text-sm font-medium">{{
      t("setup.auth.repoUrlLabel")
    }}</label>
    <BaseInput
      id="repo-url"
      v-model="repoUrl"
      type="url"
      :placeholder="t('setup.auth.repoUrlPlaceholder')"
      required
      autocomplete="off"
      :disabled="disabled"
    />
    <small class="text-xs text-muted">{{ t("setup.auth.repoUrlHint") }}</small>
  </div>

  <!-- PAT field (shown for HTTPS URLs) -->
  <div v-if="!isSshUrl" class="flex flex-col gap-1">
    <label for="pat" class="text-sm font-medium">{{
      t("setup.auth.patLabel")
    }}</label>
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
  <template v-if="isSshUrl">
    <!-- Tab toggle: Paste / Generate (hidden when showKeygen is false) -->
    <div
      v-if="showKeygen"
      class="flex gap-1 border border-edge rounded-md overflow-hidden"
    >
      <button
        type="button"
        :class="[
          'flex-1 py-2 text-sm font-medium transition-colors',
          sshKeySource === 'paste'
            ? 'bg-accent text-on-accent active:bg-accent-deep'
            : 'bg-surface active:bg-hover',
        ]"
        @click="sshKeySource = 'paste'"
      >
        {{ t("setup.auth.tabPasteKey") }}
      </button>
      <button
        type="button"
        :class="[
          'flex-1 py-2 text-sm font-medium transition-colors',
          sshKeySource === 'generate'
            ? 'bg-accent text-on-accent active:bg-accent-deep'
            : 'bg-surface active:bg-hover',
        ]"
        @click="sshKeySource = 'generate'"
      >
        {{ t("setup.auth.tabGenerateKey") }}
      </button>
    </div>

    <!-- Paste key (or always-shown SSH block when keygen hidden) -->
    <template v-if="sshKeySource === 'paste' || !showKeygen">
      <div class="flex flex-col gap-1">
        <label for="ssh-key" class="text-sm font-medium">{{
          t("setup.auth.sshKeyLabel")
        }}</label>
        <BaseTextarea
          id="ssh-key"
          v-model="sshKey"
          rows="5"
          :placeholder="t('setup.auth.sshKeyPlaceholder')"
          required
          autocomplete="off"
          spellcheck="false"
          :disabled="disabled"
        />
        <small class="text-xs text-muted">{{
          t("setup.auth.sshKeyHint")
        }}</small>
      </div>
      <div class="flex flex-col gap-1">
        <label for="ssh-passphrase" class="text-sm font-medium">{{
          t("setup.auth.sshPassphraseLabel")
        }}</label>
        <BaseInput
          id="ssh-passphrase"
          v-model="sshPassphrase"
          type="password"
          :placeholder="t('setup.auth.sshPassphrasePlaceholder')"
          autocomplete="off"
          :disabled="disabled"
        />
      </div>
    </template>

    <!-- Generate key -->
    <template v-if="showKeygen && sshKeySource === 'generate'">
      <PassphraseField
        ref="pf"
        id="ssh-gen-passphrase"
        v-model="sshPassphrase"
        :label="t('setup.auth.genPassphraseLabel')"
        :placeholder="t('setup.auth.genPassphrasePlaceholder')"
        :optional="true"
        :disabled="disabled || generating"
      />
      <BaseButton
        variant="secondary"
        :loading="generating"
        :disabled="disabled"
        @click="generateKey"
      >
        <BaseIcon v-if="!generating" :icon="KeyRound" />
        {{
          generating
            ? t("setup.auth.genButtonLoading")
            : t("setup.auth.genButton")
        }}
      </BaseButton>

      <!-- Public key display after generation -->
      <div v-if="generatedPublicKey" class="flex flex-col gap-2">
        <div class="flex items-center justify-between">
          <span
            class="text-sm font-medium text-success inline-flex items-center gap-1"
          >
            <BaseIcon :icon="CircleCheck" :size="14" />
            {{ t("setup.auth.publicKeyLabel") }}
          </span>
          <button type="button" class="btn-copy" @click="copyPublicKey">
            <BaseIcon :icon="Copy" /> {{ t("setup.auth.publicKeyCopy") }}
          </button>
        </div>
        <pre class="public-key-display" @click="copyPublicKey">{{
          generatedPublicKey
        }}</pre>
        <small class="text-xs text-muted">{{
          t("setup.auth.publicKeyHint")
        }}</small>
      </div>
    </template>
  </template>
</template>
