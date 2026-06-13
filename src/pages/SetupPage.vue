<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useRouter } from "vue-router";
import { invoke } from "@tauri-apps/api/core";
import type {
  AppError,
  IdentityInfoResult,
  RecipientInfo,
  SshKeyPairResult,
} from "../types";

const router = useRouter();

// ── Step state ──────────────────────────────────────────────────────────
const step = ref(1);

// Auto-advance to step 2 if repo is already cloned (identity missing)
onMounted(async () => {
  try {
    const ready = await invoke<boolean>("is_repo_ready");
    if (ready) {
      step.value = 2;
    }
  } catch {
    // Not ready — stay on step 1
  }
});

// ── Step 1 state: clone ─────────────────────────────────────────────────
const repoUrl = ref("");
const pat = ref("");
const sshKey = ref("");
const sshPassphrase = ref("");
const loading = ref(false);
const error = ref("");
const progressStep = ref(0);
const progressSteps = ["Cloning repository..."];
let progressTimer: ReturnType<typeof setInterval> | null = null;

// SSH key generation state
const sshKeySource = ref<"paste" | "generate">("paste");
const generatedPublicKey = ref("");
const generating = ref(false);

// ── Step 2 state: identity ──────────────────────────────────────────────
const recipients = ref<RecipientInfo[]>([]);
const selectedRecipient = ref("");
const identity = ref("");
const passphrase = ref("");
const identityType = ref<string>("");
const isIdentityEncrypted = ref(false);
const loadingRecipients = ref(false);
const loadingIdentity = ref(false);

const isSshUrl = computed(() => {
  const url = repoUrl.value.trim();
  return (
    url.startsWith("ssh://") ||
    (url.includes("@") && url.includes(":") && !url.startsWith("http"))
  );
});

const hasSshRecipients = computed(() =>
  recipients.value.some(
    (r) => r.key_type === "ssh_ed25519" || r.key_type === "ssh_rsa",
  ),
);

const canReuseSshKey = computed(
  () => isSshUrl.value && hasSshRecipients.value && sshKey.value.trim(),
);

// Whether the pasted/generated identity is an SSH key (vs. native x25519).
// Controls passphrase field semantics: SSH keys use their own passphrase to
// decrypt the key; x25519 keys use it for optional at-rest encryption.
const isSshIdentity = computed(
  () =>
    identityType.value === "ssh_ed25519" || identityType.value === "ssh_rsa",
);

function useSshKeyForIdentity() {
  identity.value = sshKey.value;
}

// ── Step 1 functions ────────────────────────────────────────────────────

function startProgress() {
  progressStep.value = 0;
  progressTimer = setInterval(() => {
    if (progressStep.value < progressSteps.length - 1) {
      progressStep.value++;
    }
  }, 2000);
}

function stopProgress() {
  if (progressTimer) {
    clearInterval(progressTimer);
    progressTimer = null;
  }
}

function validateStep1(): string | null {
  if (!repoUrl.value.trim()) return "Repository URL is required";
  const url = repoUrl.value.trim();
  const isHttps = url.startsWith("https://");
  const isSsh =
    url.startsWith("ssh://") ||
    (url.includes("@") && url.includes(":") && !url.startsWith("http"));
  if (!isHttps && !isSsh) {
    return "URL must be HTTPS or SSH format (e.g. git@host:user/repo.git)";
  }
  if (isSsh && !sshKey.value.trim()) {
    return "SSH private key is required for SSH URLs";
  }
  return null;
}

async function generateKey() {
  generating.value = true;
  error.value = "";
  try {
    const result = await invoke<SshKeyPairResult>("generate_ssh_key", {
      passphrase: sshPassphrase.value || null,
    });
    sshKey.value = result.private_key;
    generatedPublicKey.value = result.public_key;
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Key generation failed";
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

async function onClone() {
  error.value = "";
  const validationError = validateStep1();
  if (validationError) {
    error.value = validationError;
    return;
  }

  loading.value = true;
  startProgress();

  try {
    await invoke("clone_repo", {
      repoUrl: repoUrl.value,
      pat: isSshUrl.value ? null : pat.value || null,
      sshKey: isSshUrl.value ? sshKey.value : null,
      sshPassphrase: isSshUrl.value ? sshPassphrase.value || null : null,
    });
    step.value = 2;
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Clone failed";
  } finally {
    stopProgress();
    loading.value = false;
  }
}

// ── Step 2 functions ────────────────────────────────────────────────────

async function fetchRecipients() {
  loadingRecipients.value = true;
  try {
    recipients.value = await invoke<RecipientInfo[]>("list_recipients");
    // Auto-select first recipient if only one exists
    if (recipients.value.length === 1) {
      selectedRecipient.value = recipients.value[0].public_key;
    }
  } catch {
    // Recipients may not exist (empty repo) — that's fine
    recipients.value = [];
  } finally {
    loadingRecipients.value = false;
  }
}

function validateStep2(): string | null {
  if (!identity.value.trim()) return "Age identity is required";
  const trimmed = identity.value.trim();
  const isAgeKey = trimmed.startsWith("AGE-SECRET-KEY-");
  const isSshKey =
    trimmed.startsWith("-----BEGIN OPENSSH PRIVATE KEY-----") ||
    trimmed.startsWith("-----BEGIN RSA PRIVATE KEY-----");
  if (!isAgeKey && !isSshKey)
    return "Identity must be an age key (AGE-SECRET-KEY-...) or SSH private key";
  if (recipients.value.length > 0 && !selectedRecipient.value)
    return "Please select a recipient";
  // Encrypted SSH keys require their passphrase to derive a recipient.
  if (isSshIdentity.value && isIdentityEncrypted.value && !passphrase.value)
    return "SSH key passphrase is required";
  return null;
}

async function onCompleteSetup() {
  error.value = "";
  const validationError = validateStep2();
  if (validationError) {
    error.value = validationError;
    return;
  }

  loadingIdentity.value = true;
  try {
    await invoke("complete_setup", {
      identity: identity.value,
      passphrase: passphrase.value || null,
    });
    router.push({ name: "entries" });
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Setup failed";
  } finally {
    loadingIdentity.value = false;
  }
}

function goBack() {
  error.value = "";
  step.value = 1;
}

function truncateKey(key: string): string {
  if (key.length <= 24) return key;
  return `${key.slice(0, 12)}…${key.slice(-8)}`;
}

// Fetch recipients when entering step 2
watch(step, (s) => {
  if (s === 2) {
    fetchRecipients();
  }
});

// Detect identity type and SSH-key encryption status when identity changes
watch(identity, async (val) => {
  const trimmed = val.trim();
  if (trimmed.startsWith("AGE-SECRET-KEY-")) {
    identityType.value = "x25519";
    isIdentityEncrypted.value = false;
    return;
  }
  if (
    !trimmed.startsWith("-----BEGIN OPENSSH PRIVATE KEY-----") &&
    !trimmed.startsWith("-----BEGIN RSA PRIVATE KEY-----")
  ) {
    identityType.value = "";
    isIdentityEncrypted.value = false;
    return;
  }
  try {
    const info = await invoke<IdentityInfoResult>("validate_identity", {
      identity: trimmed,
    });
    identityType.value = info.key_type;
    isIdentityEncrypted.value = info.encrypted;
  } catch {
    identityType.value = "";
    isIdentityEncrypted.value = false;
  }
});
</script>

<template>
  <main
    class="min-h-screen flex items-center justify-center max-[480px]:items-start p-4 max-[480px]:pt-6 max-[480px]:pb-0"
    role="main"
  >
    <div
      class="w-full max-w-[420px] bg-surface rounded-lg p-8 shadow-[0_2px_12px_rgba(0,0,0,0.08)] max-[480px]:p-4 max-[480px]:pb-[calc(3rem+4rem)]"
    >
      <h1 class="text-center text-display mb-1">🔐 gpm</h1>
      <p class="text-center text-muted text-sm mb-6">
        Age-only gopass password client
      </p>

      <!-- Step indicator -->
      <div class="flex items-center justify-center gap-2 mb-6">
        <span
          :class="[
            'inline-flex items-center justify-center w-7 h-7 rounded-full text-xs font-bold',
            step >= 1 ? 'bg-accent text-white' : 'bg-edge text-muted',
          ]"
          >1</span
        >
        <div :class="['h-0.5 w-8', step >= 2 ? 'bg-accent' : 'bg-edge']"></div>
        <span
          :class="[
            'inline-flex items-center justify-center w-7 h-7 rounded-full text-xs font-bold',
            step >= 2 ? 'bg-accent text-white' : 'bg-edge text-muted',
          ]"
          >2</span
        >
      </div>

      <!-- ═══════ Step 1: Clone ═══════ -->
      <form
        v-if="step === 1"
        @submit.prevent="onClone"
        class="flex flex-col gap-4"
      >
        <div class="flex flex-col gap-1">
          <label for="repo-url" class="text-sm font-medium"
            >Git Repository URL</label
          >
          <input
            id="repo-url"
            v-model="repoUrl"
            type="url"
            placeholder="https://github.com/user/password-store.git"
            required
            autocomplete="off"
            :disabled="loading"
            class="input-base"
          />
          <small class="text-xs text-muted"
            >HTTPS or SSH (e.g. git@github.com:user/repo.git)</small
          >
        </div>

        <!-- PAT field (shown for HTTPS URLs) -->
        <div v-if="!isSshUrl" class="flex flex-col gap-1">
          <label for="pat" class="text-sm font-medium"
            >Personal Access Token</label
          >
          <input
            id="pat"
            v-model="pat"
            type="password"
            placeholder="Optional — for private repos"
            autocomplete="off"
            :disabled="loading"
            class="input-base"
          />
          <small class="text-xs text-muted"
            >HTTPS PAT for git authentication. Leave empty for public
            repos.</small
          >
        </div>

        <!-- SSH key fields (shown for SSH URLs) -->
        <template v-if="isSshUrl">
          <!-- Tab toggle: Paste / Generate -->
          <div
            class="flex gap-1 border border-[var(--color-edge)] rounded-[var(--radius-md)] overflow-hidden"
          >
            <button
              type="button"
              :class="[
                'flex-1 py-2 text-sm font-medium transition-colors',
                sshKeySource === 'paste'
                  ? 'bg-accent text-white'
                  : 'bg-surface',
              ]"
              @click="sshKeySource = 'paste'"
            >
              Paste Key
            </button>
            <button
              type="button"
              :class="[
                'flex-1 py-2 text-sm font-medium transition-colors',
                sshKeySource === 'generate'
                  ? 'bg-accent text-white'
                  : 'bg-surface',
              ]"
              @click="sshKeySource = 'generate'"
            >
              Generate Key
            </button>
          </div>

          <!-- Paste key -->
          <template v-if="sshKeySource === 'paste'">
            <div class="flex flex-col gap-1">
              <label for="ssh-key" class="text-sm font-medium"
                >SSH Private Key</label
              >
              <textarea
                id="ssh-key"
                v-model="sshKey"
                placeholder="-----BEGIN OPENSSH PRIVATE KEY-----&#10;..."
                rows="5"
                required
                autocomplete="off"
                spellcheck="false"
                :disabled="loading"
                class="input-base"
              />
              <small class="text-xs text-muted"
                >Paste your SSH private key (OpenSSH or PEM format)</small
              >
            </div>
            <div class="flex flex-col gap-1">
              <label for="ssh-passphrase" class="text-sm font-medium"
                >SSH Key Passphrase</label
              >
              <input
                id="ssh-passphrase"
                v-model="sshPassphrase"
                type="password"
                placeholder="Optional — if key is encrypted"
                autocomplete="off"
                :disabled="loading"
                class="input-base"
              />
            </div>
          </template>

          <!-- Generate key -->
          <template v-if="sshKeySource === 'generate'">
            <div class="flex flex-col gap-1">
              <label for="ssh-gen-passphrase" class="text-sm font-medium"
                >Key Passphrase (optional)</label
              >
              <input
                id="ssh-gen-passphrase"
                v-model="sshPassphrase"
                type="password"
                placeholder="Optional — encrypt the generated key"
                autocomplete="off"
                :disabled="loading || generating"
                class="input-base"
              />
            </div>
            <button
              type="button"
              :disabled="generating || loading"
              class="btn-secondary"
              @click="generateKey"
            >
              <span v-if="generating" class="spinner" aria-hidden="true"></span>
              <span v-if="generating">Generating...</span>
              <span v-else>🔑 Generate SSH Key</span>
            </button>

            <!-- Public key display after generation -->
            <div v-if="generatedPublicKey" class="flex flex-col gap-2">
              <div class="flex items-center justify-between">
                <span class="text-sm font-medium text-success"
                  >✓ Public Key</span
                >
                <button type="button" class="btn-copy" @click="copyPublicKey">
                  📋 Copy
                </button>
              </div>
              <pre class="public-key-display" @click="copyPublicKey">{{
                generatedPublicKey
              }}</pre>
              <small class="text-xs text-muted"
                >Add this public key to your Git provider (e.g. GitHub →
                Settings → SSH keys)</small
              >
            </div>
          </template>
        </template>

        <div
          v-if="error"
          class="bg-danger-soft text-danger p-2 px-3 rounded-sm text-sm"
          role="alert"
        >
          {{ error }}
        </div>

        <button type="submit" :disabled="loading" class="btn-primary">
          <span v-if="loading" class="spinner-white" aria-hidden="true"></span>
          <span v-if="loading">{{ progressSteps[progressStep] }}</span>
          <span v-else>Clone Repository</span>
        </button>
      </form>

      <!-- ═══════ Step 2: Identity ═══════ -->
      <form
        v-if="step === 2"
        @submit.prevent="onCompleteSetup"
        class="flex flex-col gap-4"
      >
        <!-- Back button -->
        <button
          type="button"
          class="self-start text-sm text-muted hover:text-accent transition-colors"
          @click="goBack"
        >
          ← Back
        </button>

        <h2 class="text-lg font-semibold">Select Recipient</h2>
        <p class="text-xs text-muted">
          This repository encrypts secrets to the following recipients. Select
          yours and paste the matching identity key.
        </p>

        <!-- Recipients list -->
        <div
          v-if="loadingRecipients"
          class="text-center py-4 text-sm text-muted"
        >
          Loading recipients…
        </div>

        <div v-else-if="recipients.length > 0" class="flex flex-col gap-2">
          <div
            v-for="r in recipients"
            :key="r.public_key"
            :class="[
              'flex items-start gap-3 p-3 rounded-[var(--radius-md)] border cursor-pointer transition-colors',
              selectedRecipient === r.public_key
                ? 'border-accent bg-accent-soft'
                : 'border-[var(--color-edge)] bg-[var(--color-input)] hover:bg-hover',
            ]"
            @click="selectedRecipient = r.public_key"
          >
            <input
              type="radio"
              :checked="selectedRecipient === r.public_key"
              class="mt-0.5 accent-[var(--color-accent)]"
              tabindex="-1"
              readonly
            />
            <div class="flex flex-col gap-0.5 min-w-0">
              <div class="flex items-center gap-1.5">
                <code class="text-xs font-mono break-all">{{
                  truncateKey(r.public_key)
                }}</code>
                <span
                  v-if="r.key_type !== 'x25519'"
                  class="shrink-0 text-[10px] font-medium px-1.5 py-0.5 rounded bg-[var(--color-edge)] text-muted"
                  >SSH</span
                >
              </div>
              <span v-if="r.comment" class="text-xs text-muted">{{
                r.comment
              }}</span>
            </div>
          </div>
        </div>

        <div
          v-else
          class="text-sm text-muted p-3 bg-[var(--color-input)] rounded-[var(--radius-md)]"
        >
          No recipients file found. You can still provide your identity.
        </div>

        <!-- SSH key reuse offer -->
        <button
          v-if="canReuseSshKey && !identity.trim()"
          type="button"
          class="btn-secondary text-sm"
          @click="useSshKeyForIdentity"
        >
          🔑 Use my SSH key for decryption
        </button>

        <div class="flex flex-col gap-1">
          <label for="identity" class="text-sm font-medium">Age Identity</label>
          <textarea
            id="identity"
            v-model="identity"
            placeholder="AGE-SECRET-KEY-...&#10;or paste an SSH private key"
            rows="5"
            required
            autocomplete="off"
            spellcheck="false"
            :disabled="loadingIdentity"
            class="input-base"
          />
          <small class="text-xs text-muted"
            >Paste your age secret key (AGE-SECRET-KEY-...) or SSH private
            key</small
          >
        </div>

        <!-- SSH key passphrase (required when identity is an encrypted SSH key) -->
        <div
          v-if="isSshIdentity && isIdentityEncrypted"
          class="flex flex-col gap-1"
        >
          <label for="passphrase" class="text-sm font-medium"
            >SSH Key Passphrase</label
          >
          <input
            id="passphrase"
            v-model="passphrase"
            type="password"
            placeholder="Passphrase to decrypt the SSH key"
            autocomplete="off"
            :disabled="loadingIdentity"
            class="input-base"
          />
          <small class="text-xs text-muted"
            >This SSH key is passphrase-encrypted. Enter its passphrase to use
            it as an age identity.</small
          >
        </div>

        <!-- Optional at-rest encryption (x25519 keys only; SSH keys rely on
             their own native passphrase protection) -->
        <div v-else-if="identityType === 'x25519'" class="flex flex-col gap-1">
          <label for="passphrase" class="text-sm font-medium"
            >Passphrase (optional)</label
          >
          <input
            id="passphrase"
            v-model="passphrase"
            type="password"
            placeholder="Leave empty for plaintext storage"
            autocomplete="new-password"
            :disabled="loadingIdentity"
            class="input-base"
          />
          <small class="text-xs text-muted"
            >Encrypts the identity file at rest. Recommended for Android.</small
          >
          <p
            v-if="!passphrase.trim()"
            class="text-xs text-warning bg-warning-soft p-1.5 px-2.5 rounded-sm"
          >
            ⚠ Without a passphrase, the identity is stored in plaintext.
          </p>
        </div>

        <p
          class="text-center text-xs text-info bg-info-soft p-2 px-3 rounded-sm"
        >
          Stored locally. Nothing leaves your device.
        </p>

        <div
          v-if="error"
          class="bg-danger-soft text-danger p-2 px-3 rounded-sm text-sm"
          role="alert"
        >
          {{ error }}
        </div>

        <button type="submit" :disabled="loadingIdentity" class="btn-primary">
          <span
            v-if="loadingIdentity"
            class="spinner-white"
            aria-hidden="true"
          ></span>
          <span v-if="loadingIdentity">Verifying…</span>
          <span v-else>Complete Setup</span>
        </button>
      </form>
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

.btn-primary {
  padding: 0.75rem;
  background: var(--color-accent);
  color: white;
  border: none;
  border-radius: var(--radius-md);
  font-size: var(--text-md);
  font-weight: 500;
  cursor: pointer;
  transition: background 0.2s;
  min-height: 48px;
}

.btn-primary:hover:not(:disabled) {
  background: var(--color-accent-deep);
}

.btn-primary:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.btn-secondary {
  padding: 0.75rem;
  background: var(--color-surface);
  color: inherit;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  font-size: var(--text-md);
  font-weight: 500;
  cursor: pointer;
  transition: background 0.2s;
  min-height: 48px;
}

.btn-secondary:hover:not(:disabled) {
  background: var(--color-hover);
}

.btn-secondary:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.btn-copy {
  padding: 0.3rem 0.6rem;
  font-size: var(--text-xs);
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-sm);
  background: var(--color-surface);
  cursor: pointer;
  min-height: 36px;
}

.btn-copy:hover {
  background: var(--color-hover);
}

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

.spinner-white {
  display: inline-block;
  width: 14px;
  height: 14px;
  border: 2px solid rgba(255, 255, 255, 0.3);
  border-top-color: white;
  border-radius: 50%;
  animation: spin 0.6s linear infinite;
  margin-right: 0.4rem;
  vertical-align: middle;
}

.spinner {
  display: inline-block;
  width: 14px;
  height: 14px;
  border: 2px solid var(--color-edge);
  border-top-color: var(--color-accent);
  border-radius: 50%;
  animation: spin 0.6s linear infinite;
  margin-right: 0.4rem;
  vertical-align: middle;
}
</style>
