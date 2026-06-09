<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { computed, ref } from "vue";
import { useRouter } from "vue-router";
import { invoke } from "@tauri-apps/api/core";
import type { AppError, SshKeyPairResult } from "../types";

const router = useRouter();

const repoUrl = ref("");
const pat = ref("");
const sshKey = ref("");
const sshPassphrase = ref("");
const identity = ref("");
const loading = ref(false);
const error = ref("");
const progressStep = ref(0);
const progressSteps = [
  "Cloning repository...",
  "Verifying encryption...",
  "Preparing store...",
];
let progressTimer: ReturnType<typeof setInterval> | null = null;

// SSH key generation state
const sshKeySource = ref<"paste" | "generate">("paste");
const generatedPublicKey = ref("");
const generating = ref(false);

const isSshUrl = computed(() => {
  const url = repoUrl.value.trim();
  return (
    url.startsWith("ssh://") ||
    (url.includes("@") && url.includes(":") && !url.startsWith("http"))
  );
});

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

function validate(): string | null {
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
  if (!identity.value.trim()) return "Age identity is required";
  if (!identity.value.trim().startsWith("AGE-SECRET-KEY-"))
    return "Identity must start with AGE-SECRET-KEY-...";
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

async function onSubmit() {
  error.value = "";
  const validationError = validate();
  if (validationError) {
    error.value = validationError;
    return;
  }

  loading.value = true;
  startProgress();

  try {
    await invoke("setup", {
      repoUrl: repoUrl.value,
      pat: isSshUrl.value ? null : pat.value || null,
      sshKey: isSshUrl.value ? sshKey.value : null,
      sshPassphrase: isSshUrl.value ? sshPassphrase.value || null : null,
      identity: identity.value,
    });
    router.push({ name: "entries" });
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Setup failed";
  } finally {
    stopProgress();
    loading.value = false;
  }
}
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

      <form @submit.prevent="onSubmit" class="flex flex-col gap-4">
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

        <div class="flex flex-col gap-1">
          <label for="identity" class="text-sm font-medium">Age Identity</label>
          <textarea
            id="identity"
            v-model="identity"
            placeholder="AGE-SECRET-KEY-..."
            rows="3"
            required
            autocomplete="off"
            spellcheck="false"
            :disabled="loading"
            class="input-base"
          />
          <small class="text-xs text-muted"
            >Paste your age secret key (starts with AGE-SECRET-KEY-)</small
          >
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

        <button type="submit" :disabled="loading" class="btn-primary">
          <span v-if="loading" class="spinner-white" aria-hidden="true"></span>
          <span v-if="loading">{{ progressSteps[progressStep] }}</span>
          <span v-else>Clone &amp; Setup</span>
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
