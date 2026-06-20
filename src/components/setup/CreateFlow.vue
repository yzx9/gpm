<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { computed, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type {
  AgeIdentityResult,
  AppError,
  SshKeyPairResult,
} from "../../types";
import RepoAuthFields from "./RepoAuthFields.vue";
import { isSshUrl as isSshRepoUrl, truncateKey } from "./url";
import "./forms.css";

// The generated identity (age secret or SSH private key). Held ONLY in memory —
// never persisted by the frontend; it is saved through `complete_setup` below.
const generatedIdentity = ref("");
// The public recipient derived from the identity; this is what seeds the
// store's `.age-recipients`.
const recipient = ref("");

const identityKind = ref<"age" | "ssh">("age");
const passphrase = ref("");

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

function selectKind(kind: "age" | "ssh") {
  if (identityKind.value === kind) return;
  identityKind.value = kind;
  // The generated key must match the selected type — force a re-generate.
  generatedIdentity.value = "";
  recipient.value = "";
}

async function generate() {
  generating.value = true;
  error.value = "";
  try {
    if (identityKind.value === "age") {
      const result = await invoke<AgeIdentityResult>("generate_age_identity");
      generatedIdentity.value = result.identity;
      recipient.value = result.recipient;
    } else {
      const result = await invoke<SshKeyPairResult>("generate_ssh_key", {
        passphrase: passphrase.value || null,
      });
      generatedIdentity.value = result.private_key;
      recipient.value = result.public_key;
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
    // create_store bootstraps the local repo + seeds .age-recipients + (if a
    // remote is given) records origin. It does NOT push — the first push is
    // deferred until after the identity is durable (orphan-recipient guard).
    await invoke("create_store", {
      recipient: recipient.value,
      repoUrl: hasRemote ? repoUrl.value.trim() : null,
      pat: hasRemote && !isSshUrl.value ? pat.value || null : null,
      sshKey: hasRemote && isSshUrl.value ? sshKey.value : null,
      sshPassphrase:
        hasRemote && isSshUrl.value ? sshPassphrase.value || null : null,
    });

    // Now the identity is durable — the remote may safely receive the store.
    await invoke("complete_setup", {
      identity: generatedIdentity.value,
      passphrase: passphrase.value || null,
    });

    if (hasRemote) {
      // First push. The store is fully created + configured locally; a failed
      // push (bad remote / auth) blocks navigation so the user sees it rather
      // than silently believing the store synced. Re-submitting retries the push.
      try {
        await invoke("push_repo");
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

    <!-- Identity type -->
    <div class="flex flex-col gap-1">
      <span class="text-sm font-medium">Identity type</span>
      <div
        class="flex gap-1 border border-[var(--color-edge)] rounded-[var(--radius-md)] overflow-hidden"
      >
        <button
          type="button"
          :class="[
            'flex-1 py-2 text-sm font-medium transition-colors',
            identityKind === 'age' ? 'bg-accent text-white' : 'bg-surface',
          ]"
          @click="selectKind('age')"
        >
          Age (x25519)
        </button>
        <button
          type="button"
          :class="[
            'flex-1 py-2 text-sm font-medium transition-colors',
            identityKind === 'ssh' ? 'bg-accent text-white' : 'bg-surface',
          ]"
          @click="selectKind('ssh')"
        >
          SSH (ed25519)
        </button>
      </div>
    </div>

    <!-- Passphrase (applied at generate for SSH, at-rest for age) -->
    <div class="flex flex-col gap-1">
      <label for="create-passphrase" class="text-sm font-medium"
        >Passphrase (optional)</label
      >
      <input
        id="create-passphrase"
        v-model="passphrase"
        type="password"
        placeholder="Leave empty for plaintext storage"
        autocomplete="new-password"
        :disabled="loading"
        class="input-base"
      />
      <small class="text-xs text-muted">{{
        identityKind === "ssh"
          ? "Encrypts the generated SSH key — set this before generating."
          : "Encrypts the identity at rest. Recommended for Android."
      }}</small>
    </div>

    <!-- Generate -->
    <button
      type="button"
      :disabled="generating || loading"
      class="btn-secondary"
      @click="generate"
    >
      <span v-if="generating" class="spinner" aria-hidden="true"></span>
      <span v-if="generating">Generating…</span>
      <span v-else>{{
        identityKind === "ssh" ? "🔑 Generate SSH key" : "🔑 Generate identity"
      }}</span>
    </button>

    <!-- Recipient (public key) — shown once generated. The secret identity is
         never rendered. -->
    <div v-if="recipient" class="flex flex-col gap-1">
      <span class="text-sm font-medium text-success"
        >✓ Recipient (public key)</span
      >
      <code class="public-key-display">{{ truncateKey(recipient) }}</code>
      <small class="text-xs text-muted"
        >This seeds your store's recipients file.</small
      >
    </div>

    <!-- Optional remote -->
    <div class="flex flex-col gap-3 pt-4 border-t border-[var(--color-edge)]">
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

    <p class="text-center text-xs text-info bg-info-soft p-2 px-3 rounded-sm">
      Stored locally. Nothing leaves your device.
    </p>

    <div
      v-if="error"
      class="bg-danger-soft text-danger p-2 px-3 rounded-sm text-sm"
      role="alert"
    >
      {{ error }}
    </div>

    <button type="submit" :disabled="loading" class="btn-primary">
      <span v-if="loading" class="spinner-white" aria-hidden="true"></span>
      <span v-if="loading">Creating…</span>
      <span v-else>Create Store</span>
    </button>
  </form>
</template>
