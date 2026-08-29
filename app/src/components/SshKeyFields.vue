<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<script setup lang="ts">
import type { AppError } from "@/api";
import { generateSshKey } from "@/api";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import BaseSegmentedControl from "@/components/base/BaseSegmentedControl.vue";
import BaseTextarea from "@/components/base/BaseTextarea.vue";
import PassphraseField from "@/components/PassphraseField.vue";
import { optionalPassphrase } from "@/utils/passphrase";
import { CircleCheck, Copy, KeyRound } from "@lucide/vue";
import { ref } from "vue";
import { useI18n } from "vue-i18n";

const { t } = useI18n();

// Two-way bound fields. The consumer (setup form, SSH-key page) owns the
// underlying refs and passes them via v-model.
const sshKey = defineModel<string>("sshKey", { required: true });
const sshPassphrase = defineModel<string>("sshPassphrase", { required: true });
// Optional error channel so generateKey failures surface in the parent's
// [role='alert'] region.
const error = defineModel<string>("error");

withDefaults(
  defineProps<{
    /** Show the Paste/Generate tab toggle + keygen UI. Consumers without a
     *  generate affordance pass false. */
    showKeygen?: boolean;
    /** Disable all inputs (wired to the parent's loading flag). */
    disabled?: boolean;
  }>(),
  { showKeygen: true, disabled: false },
);

// Key source: paste an existing key or generate a new one.
const sshKeySource = ref<"paste" | "generate">("paste");
// BaseSegmentedControl's generic widens T to string; cast back to the union.
function onSshKeySourceChange(selection: string): void {
  sshKeySource.value = selection as "paste" | "generate";
}
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
    // Verbatim to match every other write path (setup persist, rotation
    // save) — the passphrase that encrypts the key must be byte-identical to
    // the one later validated/persisted.
    const result = await generateSshKey(optionalPassphrase(sshPassphrase.value));
    sshKey.value = result.private_key;
    generatedPublicKey.value = result.public_key;
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("common.sshKeyFields.errKeyGen");
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

/** Clear the internal (non-model) state: generated public key, source tab,
 *  passphrase confirm field. The models themselves stay owned by the parent. */
function reset() {
  generatedPublicKey.value = "";
  sshKeySource.value = "paste";
  pf.value?.reset();
}

defineExpose({ reset });
</script>

<template>
  <!-- Tab toggle: Paste / Generate (hidden when showKeygen is false) -->
  <BaseSegmentedControl
    v-if="showKeygen"
    name="ssh-key-source"
    :model-value="sshKeySource"
    :options="[
      { label: t('common.sshKeyFields.tabPasteKey'), value: 'paste' },
      { label: t('common.sshKeyFields.tabGenerateKey'), value: 'generate' },
    ]"
    @change="onSshKeySourceChange"
  />

  <!-- Paste key (or always-shown SSH block when keygen hidden) -->
  <template v-if="sshKeySource === 'paste' || !showKeygen">
    <div class="flex flex-col gap-1">
      <label for="ssh-key" class="text-sm font-medium">{{
        t("common.sshKeyFields.sshKeyLabel")
      }}</label>
      <BaseTextarea
        id="ssh-key"
        v-model="sshKey"
        rows="5"
        :placeholder="t('common.sshKeyFields.sshKeyPlaceholder')"
        required
        autocomplete="off"
        spellcheck="false"
        :disabled="disabled"
      />
      <small class="text-xs text-muted">{{ t("common.sshKeyFields.sshKeyHint") }}</small>
    </div>
    <div class="flex flex-col gap-1">
      <label for="ssh-passphrase" class="text-sm font-medium">{{
        t("common.sshKeyFields.sshPassphraseLabel")
      }}</label>
      <BaseInput
        id="ssh-passphrase"
        v-model="sshPassphrase"
        type="password"
        :placeholder="t('common.sshKeyFields.sshPassphrasePlaceholder')"
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
      :label="t('common.sshKeyFields.genPassphraseLabel')"
      :placeholder="t('common.sshKeyFields.genPassphrasePlaceholder')"
      :optional="true"
      :disabled="disabled || generating"
    />
    <BaseButton variant="secondary" :loading="generating" :disabled="disabled" @click="generateKey">
      <BaseIcon v-if="!generating" :icon="KeyRound" />
      {{
        generating ? t("common.sshKeyFields.genButtonLoading") : t("common.sshKeyFields.genButton")
      }}
    </BaseButton>

    <!-- Public key display after generation -->
    <div v-if="generatedPublicKey" class="flex flex-col gap-2">
      <div class="flex items-center justify-between">
        <span class="text-sm font-medium text-success inline-flex items-center gap-1">
          <BaseIcon :icon="CircleCheck" :size="14" />
          {{ t("common.sshKeyFields.publicKeyLabel") }}
        </span>
        <BaseButton variant="secondary" size="xs" @click="copyPublicKey">
          <BaseIcon :icon="Copy" />
          {{ t("common.sshKeyFields.publicKeyCopy") }}
        </BaseButton>
      </div>
      <pre class="public-key-display" @click="copyPublicKey">{{ generatedPublicKey }}</pre>
      <small class="text-xs text-muted">{{ t("common.sshKeyFields.publicKeyHint") }}</small>
    </div>
  </template>
</template>

<style scoped>
/* The generated public key: a scrollable, tap-to-copy monospace box. Tapping
   the box copies (the explicit Copy button above is the primary affordance;
   the box itself is a convenience tap target). */
.public-key-display {
  padding: 0.6rem 0.75rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  background: var(--color-input);
  font-size: var(--text-xs);
  font-family: monospace;
  word-break: break-all;
  white-space: pre-wrap;
  cursor: pointer;
  max-height: 120px;
  overflow-y: auto;
  margin: 0;
}
</style>
