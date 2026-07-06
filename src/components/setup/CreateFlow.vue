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
import RepoAuthFields from "./RepoAuthFields.vue";
import { isSshUrl as isSshRepoUrl, truncateKey } from "./url";

// The public recipient of the generated identity — the only part the frontend
// ever holds. The secret identity itself lives in backend state (staged by
// `generate_identity`, consumed by `complete_setup_from_file`); it never enters
// the WebView.
const recipient = ref("");

const identityKind = ref<CreateIdentityKind>("age");
const passphrase = ref("");
// Confirm-field controller — exposes validate()/reset() for the set-new-
// passphrase check at generate (SSH) and at create (age at-rest).
const pf = ref<InstanceType<typeof PassphraseField> | null>(null);
// Forced "this age at-rest passphrase cannot be recovered" ack. SSH key-gen has
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
// Ack is only required when an age at-rest passphrase has actually been typed
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
    error.value = appError?.message || "Generation failed";
  } finally {
    generating.value = false;
  }
}

function validate(): string | null {
  if (!recipient.value) return "Generate an identity first";

  const url = repoUrl.value.trim();
  const hasAuth = Boolean(pat.value.trim() || sshKey.value.trim());
  if (!url && hasAuth) {
    return "Enter a repository URL, or clear the authentication fields for a local-only store";
  }
  if (url) {
    const isHttps = url.startsWith("https://");
    const isSsh = isSshRepoUrl(url);
    if (!isHttps && !isSsh) {
      return "URL must be HTTPS or SSH format (e.g. git@host:user/repo.git)";
    }
    if (isSsh && !sshKey.value.trim()) {
      return "SSH private key is required for SSH remote URLs";
    }
  }
  const passphraseError = pf.value?.validate() ?? null;
  if (passphraseError) return passphraseError;
  if (ackRequired.value) {
    return "Please acknowledge that this passphrase cannot be recovered.";
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
      // it (no secret crosses IPC). For SSH, reuse the passphrase that minted the
      // key (snapshot); for age, the live field (at-rest encryption).
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
          (pushError?.message || "Initial push failed") +
          " — your store is saved locally and usable; the initial sync to the remote did not complete.";
        return;
      }
    }

    emit("done");
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Create failed";
  } finally {
    loading.value = false;
  }
}
</script>

<template>
  <form @submit.prevent="onCreate" class="flex flex-col gap-4">
    <h2 class="text-lg font-semibold">Create a new store</h2>
    <p class="text-xs text-muted">
      Generate an identity and seed a brand-new, gopass-compatible age store on
      this device. A remote is optional.
    </p>
    <p class="text-xs text-muted">
      To open your repository, gpm keeps an app key on this device — your
      private key plus the git credentials.
    </p>

    <!-- Identity type -->
    <div class="flex flex-col gap-1">
      <span class="text-sm font-medium">Identity type</span>
      <div class="flex gap-1 border border-edge rounded-md overflow-hidden">
        <button
          type="button"
          :disabled="generating || loading"
          :class="[
            'flex-1 py-2 text-sm font-medium transition-colors',
            identityKind === 'age' ? 'bg-accent text-on-accent' : 'bg-surface',
          ]"
          @click="selectKind('age')"
        >
          Age (x25519)
        </button>
        <button
          type="button"
          :disabled="generating || loading"
          :class="[
            'flex-1 py-2 text-sm font-medium transition-colors',
            identityKind === 'ssh' ? 'bg-accent text-on-accent' : 'bg-surface',
          ]"
          @click="selectKind('ssh')"
        >
          SSH (ed25519)
        </button>
      </div>
    </div>

    <!-- Passphrase (applied at generate for SSH, at-rest for age) -->
    <PassphraseField
      ref="pf"
      id="create-passphrase"
      v-model="passphrase"
      label="Passphrase (optional)"
      placeholder="Leave empty for plaintext storage"
      :optional="true"
      :disabled="loading || (identityKind === 'ssh' && !!recipient)"
    >
      <template #help>
        <small class="text-xs text-muted">{{
          identityKind === "ssh"
            ? "Encrypts the generated SSH key — set this before generating."
            : "Encrypts the private key. Recommended for Android."
        }}</small>
      </template>
    </PassphraseField>

    <!-- age at-rest: forced unrecoverable ack (only once a passphrase is
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
          ? "Generating…"
          : identityKind === "ssh"
            ? "Generate SSH key"
            : "Generate identity"
      }}
    </BaseButton>

    <!-- Recipient (public key) — shown once generated. The secret identity is
         never rendered. -->
    <div v-if="recipient" class="flex flex-col gap-1">
      <span
        class="text-sm font-medium text-success inline-flex items-center gap-1"
      >
        <BaseIcon :icon="CircleCheck" :size="14" /> Recipient (public key)
      </span>
      <code class="public-key-display">{{ truncateKey(recipient) }}</code>
      <small class="text-xs text-muted"
        >This seeds your store's recipients file.</small
      >
    </div>

    <!-- Optional remote -->
    <div class="flex flex-col gap-3 pt-4 border-t border-edge">
      <div>
        <span class="text-sm font-medium">Remote (optional)</span>
        <p class="text-xs text-muted">
          Add a git remote to sync across devices. Without one the store is
          local-only and can be synced later.
        </p>
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
      Stored locally. Nothing leaves your device.
    </BaseAlert>

    <BaseAlert v-if="error" variant="danger">{{ error }}</BaseAlert>

    <BaseButton
      variant="primary"
      type="submit"
      :loading="loading"
      :disabled="ackRequired"
      >{{ loading ? "Creating…" : "Create Store" }}</BaseButton
    >
  </form>
</template>
