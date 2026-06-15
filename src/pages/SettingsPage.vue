<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { invoke } from "@tauri-apps/api/core";
import {
  disableBiometricUnlock,
  enableBiometricUnlock,
  isBiometricAvailable,
  isBiometricUnlockEnabled,
} from "../biometric";
import type {
  AppError,
  AuthState,
  AuthenticityConfig,
  BiometricError,
  RepoConfig,
  SshPublicKeyResult,
  SshPrivateKeyResult,
  VerifyMode,
} from "../types";

const router = useRouter();

const config = ref<RepoConfig | null>(null);
const loading = ref(false);
const error = ref("");
const publicKey = ref("");
const showPublic = ref(false);
const privateKey = ref("");
const showPrivate = ref(false);
const toast = ref("");
let toastTimer: ReturnType<typeof setTimeout> | null = null;

const isSsh = ref(false);

// ── Passphrase management state ──────────────────────────────────────────
const isIdentityEncrypted = ref(false);
const identityType = ref("");
const showSetPassphrase = ref(false);
const showChangePassphrase = ref(false);
const newPassphrase = ref("");
const oldPassphrase = ref("");
const passphraseLoading = ref(false);

// ── Biometric unlock state ──────────────────────────────────────────────
const biometricAvailable = ref(false);
const biometricEnabled = ref(false);
const biometricPassphrase = ref("");
const biometricLoading = ref(false);

// ── Repository authenticity state ────────────────────────────────────────
const authConfig = ref<AuthenticityConfig | null>(null);
const authLoading = ref(false);
const showAddKey = ref(false);
const newPublicKey = ref("");
const newKeyLabel = ref("");

// Whether the stored identity is an SSH key. SSH keys are never
// passphrase-encrypted by gpm (they rely on their own native protection),
// so the at-rest encryption UI is hidden for them.
const isSshIdentity = computed(
  () =>
    identityType.value === "ssh_ed25519" || identityType.value === "ssh_rsa",
);

function showToast(message: string) {
  toast.value = message;
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    toast.value = "";
    toastTimer = null;
  }, 3000);
}

async function loadConfig() {
  loading.value = true;
  error.value = "";
  try {
    config.value = await invoke<RepoConfig>("get_config");
    isSsh.value = config.value.ssh_key !== null;
    const auth = await invoke<AuthState>("get_auth_state");
    isIdentityEncrypted.value = auth.encrypted;
    identityType.value = auth.identity_type;
    biometricAvailable.value = await isBiometricAvailable();
    biometricEnabled.value = await isBiometricUnlockEnabled();
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Failed to load config";
  } finally {
    loading.value = false;
  }
}

async function showPublicKey() {
  error.value = "";
  try {
    const result = await invoke<SshPublicKeyResult>("get_ssh_public_key");
    publicKey.value = result.public_key;
    showPublic.value = true;
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Failed to get public key";
  }
}

async function exportPrivateKey() {
  if (
    !confirm(
      "This will display your private SSH key. Make sure no one is watching. Continue?",
    )
  )
    return;
  error.value = "";
  try {
    const result = await invoke<SshPrivateKeyResult>("export_ssh_private_key");
    privateKey.value = result.private_key;
    showPrivate.value = true;
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Failed to export private key";
  }
}

async function copyText(text: string) {
  try {
    await navigator.clipboard.writeText(text);
    showToast("✓ Copied to clipboard");
  } catch {
    showToast("Copy failed");
  }
}

async function onSetPassphrase() {
  error.value = "";
  if (!newPassphrase.value) {
    error.value = "Passphrase must not be empty";
    return;
  }
  passphraseLoading.value = true;
  try {
    await invoke("set_passphrase", { passphrase: newPassphrase.value });
    isIdentityEncrypted.value = true;
    showSetPassphrase.value = false;
    newPassphrase.value = "";
    showToast("✓ Passphrase set — identity is now encrypted");
    // Setting a passphrase invalidates any sealed biometric passphrase.
    biometricEnabled.value = await isBiometricUnlockEnabled();
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Failed to set passphrase";
  } finally {
    passphraseLoading.value = false;
  }
}

async function onChangePassphrase() {
  error.value = "";
  if (!oldPassphrase.value || !newPassphrase.value) {
    error.value = "Both passphrases are required";
    return;
  }
  passphraseLoading.value = true;
  try {
    await invoke("change_passphrase", {
      oldPassphrase: oldPassphrase.value,
      newPassphrase: newPassphrase.value,
    });
    showChangePassphrase.value = false;
    oldPassphrase.value = "";
    newPassphrase.value = "";
    showToast("✓ Passphrase changed");
    // Changing the passphrase invalidates any sealed biometric passphrase.
    biometricEnabled.value = await isBiometricUnlockEnabled();
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Failed to change passphrase";
  } finally {
    passphraseLoading.value = false;
  }
}

async function onEnableBiometric() {
  error.value = "";
  if (!biometricPassphrase.value) {
    error.value = "Passphrase is required";
    return;
  }
  biometricLoading.value = true;
  try {
    await enableBiometricUnlock(biometricPassphrase.value);
    biometricEnabled.value = true;
    biometricPassphrase.value = "";
    showToast("✓ Biometric unlock enabled");
  } catch (e) {
    const err = e as BiometricError;
    if (err.code === "WRONG_PASSPHRASE") {
      error.value = "Wrong passphrase";
    } else if (err.code === "BIOMETRIC_CANCELLED") {
      // User cancelled the enable prompt — no error toast.
    } else {
      error.value = err.message || "Failed to enable biometric";
    }
  } finally {
    biometricLoading.value = false;
  }
}

async function onDisableBiometric() {
  await disableBiometricUnlock();
  biometricEnabled.value = false;
  showToast("Biometric unlock disabled");
}

// ── Repository authenticity ──────────────────────────────────────────────
async function loadAuthConfig() {
  try {
    authConfig.value = await invoke<AuthenticityConfig>(
      "get_authenticity_config",
    );
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Failed to load authenticity config";
  }
}

async function onModeChange(mode: VerifyMode) {
  if (!authConfig.value) return;
  authLoading.value = true;
  error.value = "";
  try {
    const effective = await invoke<VerifyMode>("set_verification_mode", {
      mode,
    });
    authConfig.value.mode = effective;
  } catch (e) {
    const appError = e as AppError;
    if (mode === "enforce") {
      error.value =
        "Add a trusted signing key before enabling Enforce (it would block every pull).";
      // Revert the radio to the current effective mode.
      authConfig.value.mode = authConfig.value.mode;
    } else {
      error.value = appError?.message || "Failed to set mode";
    }
  } finally {
    authLoading.value = false;
  }
}

async function onAddKey() {
  error.value = "";
  const key = newPublicKey.value.trim();
  if (!key) {
    error.value = "Paste an SSH signing public key";
    return;
  }
  authLoading.value = true;
  try {
    await invoke("add_trusted_key", {
      publicKey: key,
      label: newKeyLabel.value.trim() || "signer",
    });
    newPublicKey.value = "";
    newKeyLabel.value = "";
    showAddKey.value = false;
    showToast("✓ Trusted signing key added");
    await loadAuthConfig();
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Failed to add key";
  } finally {
    authLoading.value = false;
  }
}

async function onRemoveKey(fingerprint: string) {
  if (!confirm("Remove this trusted signing key?")) return;
  authLoading.value = true;
  try {
    await invoke("remove_trusted_key", { fingerprint });
    showToast("Trusted key removed");
    await loadAuthConfig();
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Failed to remove key";
  } finally {
    authLoading.value = false;
  }
}

async function onTrustHead() {
  const label = window.prompt(
    "Trust this repo's current signer?\nEnter a label:",
    "signer",
  );
  if (label === null) return;
  authLoading.value = true;
  try {
    await invoke("trust_head_signer", { label: label.trim() || "signer" });
    showToast("✓ HEAD signer trusted");
    await loadAuthConfig();
  } catch (e) {
    const appError = e as AppError;
    error.value =
      appError?.message || "HEAD is not SSH-signed — paste a key instead";
  } finally {
    authLoading.value = false;
  }
}

function openHistory() {
  router.push({ name: "history" });
}

async function resetConfig() {
  if (!confirm("Reset gpm? This will remove all local data and configuration."))
    return;
  try {
    await invoke("reset_config");
    router.push({ name: "setup" });
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Reset failed";
  }
}

function goBack() {
  router.push({ name: "entries" });
}

onMounted(() => {
  loadConfig();
  loadAuthConfig();
});
</script>

<template>
  <main class="max-w-[480px] md:max-w-[600px] mx-auto p-4" role="main">
    <header class="flex justify-between items-center mb-6" role="banner">
      <h1 class="text-xl">⚙ Settings</h1>
      <button class="btn-sm" @click="goBack" aria-label="Back to entries">
        ← Back
      </button>
    </header>

    <div v-if="loading" class="text-center text-muted py-8">Loading...</div>

    <div
      v-else-if="error"
      class="bg-danger-soft text-danger p-2 px-3 rounded-sm text-sm mb-4"
      role="alert"
    >
      {{ error }}
    </div>

    <div v-else-if="config" class="flex flex-col gap-4">
      <!-- Repo info -->
      <section class="settings-card">
        <h2 class="text-sm font-medium mb-2">Repository</h2>
        <div class="text-sm text-muted break-all">{{ config.url }}</div>
        <div class="text-xs text-subtle mt-1">
          Auth: {{ isSsh ? "SSH Key" : config.pat ? "PAT" : "None (public)" }}
        </div>
      </section>

      <!-- SSH key management -->
      <section v-if="isSsh" class="settings-card">
        <h2 class="text-sm font-medium mb-3">SSH Key</h2>

        <!-- Show public key -->
        <div class="flex flex-col gap-2">
          <button type="button" class="btn-action" @click="showPublicKey">
            🔑 Show Public Key
          </button>

          <div v-if="showPublic" class="mt-2 flex flex-col gap-2">
            <div class="flex justify-between items-center">
              <span class="text-xs text-muted">Public key</span>
              <button class="btn-copy" @click="copyText(publicKey)">
                📋 Copy
              </button>
            </div>
            <pre class="key-display">{{ publicKey }}</pre>
          </div>
        </div>

        <!-- Export private key -->
        <div class="flex flex-col gap-2 mt-3">
          <button
            type="button"
            class="btn-action btn-action-danger"
            @click="exportPrivateKey"
          >
            🔓 Export Private Key
          </button>

          <div v-if="showPrivate" class="mt-2 flex flex-col gap-2">
            <div
              class="bg-danger-soft text-danger p-2 px-3 rounded-sm text-xs"
              role="alert"
            >
              ⚠ Private key is now visible. Copy it to a safe place and close
              this screen.
            </div>
            <div class="flex justify-end">
              <button class="btn-copy" @click="copyText(privateKey)">
                📋 Copy
              </button>
            </div>
            <pre class="key-display private-key-display">{{ privateKey }}</pre>
            <button
              type="button"
              class="btn-action mt-1"
              @click="
                showPrivate = false;
                privateKey = '';
              "
            >
              Hide Private Key
            </button>
          </div>
        </div>
      </section>

      <!-- Passphrase management (x25519 identities only — SSH keys rely on
           their own native passphrase protection) -->
      <section v-if="!isSshIdentity" class="settings-card">
        <h2 class="text-sm font-medium mb-3">Identity Encryption</h2>

        <!-- Not encrypted: set passphrase -->
        <template v-if="!isIdentityEncrypted">
          <p class="text-xs text-muted mb-2">
            The identity is stored in plaintext. Set a passphrase to encrypt it.
          </p>
          <button
            v-if="!showSetPassphrase"
            type="button"
            class="btn-action"
            @click="showSetPassphrase = true"
          >
            🔒 Set Passphrase
          </button>
          <div v-if="showSetPassphrase" class="flex flex-col gap-2">
            <input
              v-model="newPassphrase"
              type="password"
              placeholder="New passphrase"
              autocomplete="new-password"
              class="input-base"
              :disabled="passphraseLoading"
            />
            <button
              type="button"
              class="btn-action"
              :disabled="passphraseLoading"
              @click="onSetPassphrase"
            >
              <span
                v-if="passphraseLoading"
                class="spinner"
                aria-hidden="true"
              ></span>
              Encrypt Identity
            </button>
          </div>
        </template>

        <!-- Encrypted: change passphrase -->
        <template v-else>
          <p class="text-xs text-muted mb-2">
            ✓ Identity is passphrase-encrypted.
          </p>
          <button
            v-if="!showChangePassphrase"
            type="button"
            class="btn-action"
            @click="showChangePassphrase = true"
          >
            🔑 Change Passphrase
          </button>
          <div v-if="showChangePassphrase" class="flex flex-col gap-2">
            <input
              v-model="oldPassphrase"
              type="password"
              placeholder="Current passphrase"
              autocomplete="current-password"
              class="input-base"
              :disabled="passphraseLoading"
            />
            <input
              v-model="newPassphrase"
              type="password"
              placeholder="New passphrase"
              autocomplete="new-password"
              class="input-base"
              :disabled="passphraseLoading"
            />
            <button
              type="button"
              class="btn-action"
              :disabled="passphraseLoading"
              @click="onChangePassphrase"
            >
              <span
                v-if="passphraseLoading"
                class="spinner"
                aria-hidden="true"
              ></span>
              Change Passphrase
            </button>
          </div>
        </template>
      </section>

      <!-- SSH key identities are not encrypted by gpm -->
      <section v-else class="settings-card">
        <h2 class="text-sm font-medium mb-3">Identity Encryption</h2>
        <p class="text-xs text-muted">
          SSH key identities rely on their own native passphrase protection and
          are not re-encrypted by gpm.
        </p>
      </section>

      <!-- Biometric unlock (only meaningful when the identity is encrypted) -->
      <section v-if="isIdentityEncrypted" class="settings-card">
        <h2 class="text-sm font-medium mb-3">Biometric Unlock</h2>

        <p v-if="!biometricAvailable" class="text-xs text-muted">
          Biometric unlock isn't available on this device (requires Android 11+
          with a fingerprint or face enrolled).
        </p>

        <template v-else-if="!biometricEnabled">
          <p class="text-xs text-muted mb-2">
            Unlock with fingerprint or face instead of typing your passphrase on
            every launch.
          </p>
          <div class="flex flex-col gap-2">
            <input
              v-model="biometricPassphrase"
              type="password"
              placeholder="Current passphrase"
              autocomplete="current-password"
              class="input-base"
              :disabled="biometricLoading"
            />
            <button
              type="button"
              class="btn-action"
              :disabled="biometricLoading"
              @click="onEnableBiometric"
            >
              <span
                v-if="biometricLoading"
                class="spinner"
                aria-hidden="true"
              ></span>
              Enable Biometric
            </button>
          </div>
        </template>

        <template v-else>
          <p class="text-xs text-muted mb-2">✓ Biometric unlock is enabled.</p>
          <button
            type="button"
            class="btn-action btn-action-danger"
            @click="onDisableBiometric"
          >
            Disable Biometric
          </button>
        </template>
      </section>

      <!-- Repository authenticity -->
      <section v-if="authConfig" class="settings-card">
        <h2 class="text-sm font-medium mb-3">Repository Authenticity</h2>
        <p class="text-xs text-muted mb-3">
          Verify SSH-signed commits on every pull to detect a compromised remote
          feeding validly encrypted but wrong entries.
        </p>

        <!-- Mode selector -->
        <fieldset class="border-0 p-0 m-0 mb-3" :disabled="authLoading">
          <legend class="text-xs text-muted mb-1">Verification</legend>
          <div class="flex gap-2">
            <label
              v-for="m in ['off', 'audit', 'enforce'] as VerifyMode[]"
              :key="m"
              class="mode-pill"
              :class="{ 'mode-active': authConfig.mode === m }"
            >
              <input
                type="radio"
                name="verify-mode"
                class="sr-only"
                :checked="authConfig.mode === m"
                @change="onModeChange(m)"
              />
              <span class="capitalize">{{ m }}</span>
            </label>
          </div>
          <p class="text-xs text-subtle mt-1">
            <template v-if="authConfig.mode === 'off'"
              >No verification.</template
            >
            <template v-else-if="authConfig.mode === 'audit'"
              >Verify and warn, but always pull.</template
            >
            <template v-else>Block pulls with unverified commits.</template>
          </p>
        </fieldset>

        <!-- Trusted signing keys -->
        <div class="text-xs text-muted mb-1">
          Trusted signing keys ({{ authConfig.trusted_keys.length }})
        </div>
        <ul
          v-if="authConfig.trusted_keys.length"
          class="flex flex-col gap-1 mb-2"
        >
          <li
            v-for="k in authConfig.trusted_keys"
            :key="k.fingerprint"
            class="key-row"
          >
            <code class="text-xs break-all flex-1">{{ k.fingerprint }}</code>
            <span class="text-xs text-muted mx-2 truncate">{{ k.label }}</span>
            <button
              type="button"
              class="btn-copy"
              @click="onRemoveKey(k.fingerprint)"
            >
              Remove
            </button>
          </li>
        </ul>
        <p v-else class="text-xs text-subtle mb-2">
          No trusted keys yet. Trust this repo's signer or paste a key below.
        </p>

        <div class="flex flex-col gap-2">
          <button
            v-if="authConfig.trusted_keys.length === 0"
            type="button"
            class="btn-action"
            @click="onTrustHead"
          >
            🔑 Trust this repo's signer (HEAD)
          </button>
          <button
            v-if="!showAddKey"
            type="button"
            class="btn-action"
            @click="showAddKey = true"
          >
            + Add a signing public key
          </button>
          <div v-if="showAddKey" class="flex flex-col gap-2">
            <textarea
              v-model="newPublicKey"
              placeholder="ssh-ed25519 AAAA… [comment]"
              rows="2"
              class="input-base font-mono text-xs"
            ></textarea>
            <input
              v-model="newKeyLabel"
              type="text"
              placeholder="Label (e.g. Alice — laptop)"
              class="input-base"
            />
            <div class="flex gap-2">
              <button
                type="button"
                class="btn-action flex-1"
                :disabled="authLoading"
                @click="onAddKey"
              >
                Save key
              </button>
              <button
                type="button"
                class="btn-action flex-1"
                @click="
                  showAddKey = false;
                  newPublicKey = '';
                  newKeyLabel = '';
                "
              >
                Cancel
              </button>
            </div>
          </div>
          <button type="button" class="btn-action" @click="openHistory">
            📜 View commit history &amp; signatures
          </button>
        </div>
      </section>

      <!-- Danger zone -->
      <section class="settings-card settings-card-danger">
        <h2 class="text-sm font-medium mb-2 text-danger">Danger Zone</h2>
        <button
          type="button"
          class="btn-action btn-action-danger"
          @click="resetConfig"
        >
          🗑 Reset All Data
        </button>
        <p class="text-xs text-subtle mt-1">
          Remove all local data and configuration.
        </p>
      </section>
    </div>

    <!-- Toast -->
    <div
      v-if="toast"
      class="fixed bottom-4 left-1/2 -translate-x-1/2 bg-success-soft text-success p-2 px-4 rounded-md text-sm shadow-lg z-50"
      role="status"
      aria-live="polite"
    >
      {{ toast }}
    </div>
  </main>
</template>

<style scoped>
.settings-card {
  padding: 1rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  background: var(--color-surface);
}

.settings-card-danger {
  border-color: var(--color-danger-edge, var(--color-danger, #c66));
}

.btn-sm {
  padding: 0.3rem 0.6rem;
  font-size: var(--text-xs);
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-sm);
  background: var(--color-surface);
  color: inherit;
  cursor: pointer;
  min-height: 48px;
}

.btn-sm:hover {
  background: var(--color-hover);
}

.btn-action {
  padding: 0.5rem 0.75rem;
  font-size: var(--text-sm);
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  background: var(--color-surface);
  color: inherit;
  cursor: pointer;
  min-height: 48px;
  width: 100%;
  text-align: left;
}

.btn-action:hover {
  background: var(--color-hover);
}

.btn-action-danger {
  border-color: var(--color-danger-edge, var(--color-danger, #c66));
  color: #c66;
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

.key-display {
  padding: 0.6rem 0.75rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  background: var(--color-input);
  font-size: var(--text-xs);
  font-family: monospace;
  word-break: break-all;
  white-space: pre-wrap;
  max-height: 150px;
  overflow-y: auto;
  margin: 0;
}

.private-key-display {
  max-height: 250px;
}

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

.sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
}

.mode-pill {
  flex: 1;
  text-align: center;
  padding: 0.5rem 0.6rem;
  font-size: var(--text-sm);
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  background: var(--color-surface);
  cursor: pointer;
  min-height: 48px;
  display: flex;
  align-items: center;
  justify-content: center;
}
.mode-pill:hover {
  background: var(--color-hover);
}
.mode-active {
  border-color: var(--color-accent);
  box-shadow: 0 0 0 2px var(--color-accent-ring);
}

.key-row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.4rem 0.5rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-sm);
  background: var(--color-input);
}
</style>
