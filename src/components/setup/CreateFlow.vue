<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import {
  clearPendingIdentity,
  completeSetupFromFile,
  createStore,
  generateIdentity,
  isConfigured,
  pushRepo,
  type AppError,
  type CreateIdentityKind,
} from "@/api";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import PassphraseField from "@/components/PassphraseField.vue";
import PassphraseUnrecoverableAck from "@/components/PassphraseUnrecoverableAck.vue";
import { CircleCheck, KeyRound } from "@lucide/vue";
import { computed, onUnmounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import RepoAuthFields from "./RepoAuthFields.vue";
import { isSshUrl as isSshRepoUrl, truncateKey } from "./url";

const { t } = useI18n();

// The public recipient of the generated identity — the only part the frontend
// ever holds. The secret identity itself lives in backend state (staged by
// `generate_identity`, consumed by `complete_setup_from_file`); it never enters
// the WebView.
const recipient = ref("");

const identityKind = ref<CreateIdentityKind>("age");
const passphrase = ref("");
// Confirm-field controller — exposes validate()/reset() for the set-new-
// passphrase check at generate (SSH) and at create (age seal).
const pf = ref<InstanceType<typeof PassphraseField> | null>(null);
// Forced "this age seal passphrase cannot be recovered" ack. SSH key-gen has
// its own native protection and is explicitly out of scope. Reset on every
// identity-kind switch (see selectKind) so an age ack can't leak into ssh.
const ackAge = ref(false);
// The passphrase that minted the SSH key (snapshot at generate time). SSH
// derives its recipient from the passphrase-encrypted PEM, so complete must
// reuse exactly this value — not the live field, which is locked after generate
// but could still diverge during the in-flight generate window.
const mintedSshPassphrase = ref<string | null>(null);

// Optional remote (local-first: a remote is not required to create).
const repoUrl = ref("");
const pat = ref("");
const sshKey = ref("");
const sshPassphrase = ref("");

const generating = ref(false);
const loading = ref(false);
const error = ref("");

const emit = defineEmits<{ done: [] }>();

const isSshUrl = computed(() => isSshRepoUrl(repoUrl.value));
// Ack is only required when an age seal passphrase has actually been typed
// (empty optional = plaintext = no lockout risk).
const ackRequired = computed(
  () => identityKind.value === "age" && !!passphrase.value && !ackAge.value,
);
// Invalidate the ack when the typed passphrase changes — each distinct
// committed value gets its own acknowledgment. selectKind() still resets it
// explicitly for the age↔ssh switch (passphrase itself isn't cleared there).
watch(passphrase, () => {
  ackAge.value = false;
});

function selectKind(kind: CreateIdentityKind) {
  if (identityKind.value === kind) return;
  identityKind.value = kind;
  // The staged identity + the SSH mint passphrase must match the selected type
  // — drop both so stale values can't be saved, and force a re-generate.
  recipient.value = "";
  mintedSshPassphrase.value = null;
  // The confirm field was given in the previous kind's context — clear it so a
  // stale confirm can't silently match under the new semantics. Same for the
  // age unrecoverable-ack: an ack given under one identity kind must not carry
  // into the other.
  pf.value?.reset();
  ackAge.value = false;
  clearPendingIdentity().catch(() => {});
}

// Drop any staged identity if the user leaves without completing (no-op after a
// successful complete_setup_from_file, which consumes it).
onUnmounted(() => {
  clearPendingIdentity().catch(() => {});
});

async function generate() {
  error.value = "";
  // SSH bakes the passphrase into the key at mint time — a typo there can't be
  // caught later, so require a matching confirm before minting anything.
  if (identityKind.value === "ssh") {
    const passphraseError = pf.value?.validate() ?? null;
    if (passphraseError) {
      error.value = passphraseError;
      return;
    }
  }
  generating.value = true;
  try {
    // The backend mints + stages the secret; only the recipient comes back.
    if (identityKind.value === "ssh") {
      // SSH derives its recipient from the passphrase-encrypted PEM, so the
      // passphrase used at complete must be the one that minted the key.
      // Snapshot it now (the field is also locked after generate) so a later
      // edit — or a mid-generate keystroke before the lock takes effect — can't
      // desync the two.
      mintedSshPassphrase.value = passphrase.value || null;
      recipient.value = await generateIdentity(
        "ssh",
        mintedSshPassphrase.value,
      );
    } else {
      recipient.value = await generateIdentity("age", null);
    }
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("setup.create.err.errGeneration");
  } finally {
    generating.value = false;
  }
}

function validate(): string | null {
  if (!recipient.value) return t("setup.create.validation.errGenerateFirst");

  const url = repoUrl.value.trim();
  const hasAuth = Boolean(pat.value.trim() || sshKey.value.trim());
  if (!url && hasAuth) {
    return t("setup.create.validation.errUrlOrClearAuth");
  }
  if (url) {
    const isHttps = url.startsWith("https://");
    const isSsh = isSshRepoUrl(url);
    if (!isHttps && !isSsh) {
      return t("setup.create.validation.errUrlFormat");
    }
    if (isSsh && !sshKey.value.trim()) {
      return t("setup.create.validation.errSshKeyRequired");
    }
  }
  const passphraseError = pf.value?.validate() ?? null;
  if (passphraseError) return passphraseError;
  if (ackRequired.value) {
    return t("setup.create.validation.errAckRequired");
  }
  return null;
}

async function onCreate() {
  error.value = "";
  const validationError = validate();
  if (validationError) {
    error.value = validationError;
    return;
  }

  loading.value = true;
  try {
    const hasRemote = repoUrl.value.trim().length > 0;
    // A store that's already configured (e.g. retrying after a non-fatal push
    // failure) must NOT be re-bootstrapped: create_store clears config +
    // rm -rf's the repo, and the staged identity is already consumed, so a
    // re-run would destroy the saved identity and strand the store. When the
    // store is complete, skip straight to the (retry) push.
    const configured = await isConfigured();
    if (!configured) {
      // create_store bootstraps the local repo + seeds .age-recipients + (if a
      // remote is given) records origin. It does NOT push — the first push is
      // deferred until after the identity is durable (orphan-recipient guard).
      await createStore(
        recipient.value,
        hasRemote ? repoUrl.value.trim() : null,
        hasRemote && !isSshUrl.value ? pat.value || null : null,
        hasRemote && isSshUrl.value ? sshKey.value : null,
        hasRemote && isSshUrl.value ? sshPassphrase.value || null : null,
      );

      // The identity was staged in backend state at generate time; this consumes
      // it (no secret crosses IPC). For SSH, reuse the passphrase that minted
      // the key (snapshot); for age, the live field (seal encryption).
      await completeSetupFromFile(
        identityKind.value === "ssh"
          ? mintedSshPassphrase.value
          : passphrase.value || null,
      );
    }

    if (hasRemote) {
      // First push — or, after a prior push failure, the retry. The store is
      // fully created + configured locally; a failed push blocks navigation so
      // the user sees it rather than silently believing the store synced.
      try {
        await pushRepo();
      } catch (e) {
        const pushError = e as AppError;
        error.value =
          (pushError?.message || t("setup.create.err.errPush")) +
          t("setup.create.err.errPushSuffix");
        return;
      }
    }

    emit("done");
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("setup.create.err.errCreate");
  } finally {
    loading.value = false;
  }
}
</script>

<template>
  <form @submit.prevent="onCreate" class="flex flex-col gap-4">
    <h2 class="text-lg font-semibold">{{ t("setup.create.heading") }}</h2>
    <p class="text-xs text-muted">{{ t("setup.create.intro") }}</p>
    <p class="text-xs text-muted">{{ t("common.setup.introAppKey") }}</p>

    <!-- Identity type -->
    <div class="flex flex-col gap-1">
      <span class="text-sm font-medium">{{
        t("setup.create.identityTypeLabel")
      }}</span>
      <div class="flex gap-1 border border-edge rounded-md overflow-hidden">
        <button
          type="button"
          :disabled="generating || loading"
          :class="[
            'flex-1 py-2 text-sm font-medium transition-colors active:bg-hover',
            identityKind === 'age' ? 'bg-accent text-on-accent' : 'bg-surface',
          ]"
          @click="selectKind('age')"
        >
          {{ t("setup.create.tabAge") }}
        </button>
        <button
          type="button"
          :disabled="generating || loading"
          :class="[
            'flex-1 py-2 text-sm font-medium transition-colors active:bg-hover',
            identityKind === 'ssh' ? 'bg-accent text-on-accent' : 'bg-surface',
          ]"
          @click="selectKind('ssh')"
        >
          {{ t("setup.create.tabSsh") }}
        </button>
      </div>
    </div>

    <!-- Passphrase (applied at generate for SSH, seal for age) -->
    <PassphraseField
      ref="pf"
      id="create-passphrase"
      v-model="passphrase"
      :label="t('setup.create.passphraseLabel')"
      :placeholder="t('setup.create.passphrasePlaceholder')"
      :optional="true"
      :disabled="loading || (identityKind === 'ssh' && !!recipient)"
    >
      <template #help>
        <small class="text-xs text-muted">{{
          identityKind === "ssh"
            ? t("setup.create.passphraseHelpSsh")
            : t("setup.create.passphraseHelpAge")
        }}</small>
      </template>
    </PassphraseField>

    <!-- age seal: forced unrecoverable ack (only once a passphrase is
         typed; empty = plaintext = no lockout risk). SSH key-gen is out of
         scope — its passphrase uses SSH's own native protection. -->
    <PassphraseUnrecoverableAck
      v-if="identityKind === 'age' && passphrase"
      v-model="ackAge"
    />

    <!-- Generate -->
    <BaseButton
      variant="secondary"
      :loading="generating"
      :disabled="loading"
      @click="generate"
    >
      <BaseIcon v-if="!generating" :icon="KeyRound" />
      {{
        generating
          ? t("setup.create.generating")
          : identityKind === "ssh"
            ? t("setup.create.generateSsh")
            : t("setup.create.generate")
      }}
    </BaseButton>

    <!-- Recipient (public key) — shown once generated. The secret identity is
         never rendered. -->
    <div v-if="recipient" class="flex flex-col gap-1">
      <span
        class="text-sm font-medium text-success inline-flex items-center gap-1"
      >
        <BaseIcon :icon="CircleCheck" :size="14" />
        {{ t("setup.create.recipientLabel") }}
      </span>
      <code class="public-key-display">{{ truncateKey(recipient) }}</code>
      <small class="text-xs text-muted">{{
        t("setup.create.recipientHint")
      }}</small>
    </div>

    <!-- Optional remote -->
    <div class="flex flex-col gap-3 pt-4 border-t border-edge">
      <div>
        <span class="text-sm font-medium">{{
          t("setup.create.remoteLabel")
        }}</span>
        <p class="text-xs text-muted">{{ t("setup.create.remoteHint") }}</p>
      </div>
      <div class="flex flex-col gap-4">
        <RepoAuthFields
          v-model:repo-url="repoUrl"
          v-model:pat="pat"
          v-model:ssh-key="sshKey"
          v-model:ssh-passphrase="sshPassphrase"
          :show-keygen="false"
          :disabled="loading"
        />
      </div>
    </div>

    <BaseAlert variant="info" class="text-center">
      {{ t("common.setup.storedLocally") }}
    </BaseAlert>

    <BaseAlert v-if="error" variant="danger">{{ error }}</BaseAlert>

    <BaseButton
      variant="primary"
      type="submit"
      :loading="loading"
      :disabled="ackRequired"
      >{{
        loading ? t("setup.create.creating") : t("setup.create.buttonCreate")
      }}</BaseButton
    >
  </form>
</template>
