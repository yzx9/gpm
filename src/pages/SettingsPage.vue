<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import type {
  AppError,
  AppLockError,
  AuthenticityConfig,
  BiometricError,
  CommitIdentity,
  LockMode,
  RepoConfig,
  VerifyMode,
} from "@/api";
import {
  addTrustedKey,
  resetConfig as apiResetConfig,
  asAppLockError,
  changePassphrase,
  disableBiometricAppLock,
  disableBiometricUnlock,
  disableIdentityAutoUnlock,
  enableBiometricAppLock,
  enableBiometricUnlock,
  enableIdentityAutoUnlock,
  exportSshPrivateKey,
  getAuthState,
  getAuthenticityConfig,
  getCommitIdentityDefault,
  getConfig,
  getSshPublicKey,
  isAppLockAvailable,
  isBiometricAvailable,
  isBiometricUnlockEnabled,
  removeTrustedKey,
  setAutosync,
  setClipboardClearSecs,
  setCommitIdentity,
  setLockMode,
  setPassphrase,
  setVerificationMode,
  setViewClearSecs,
  trustHeadSigner,
} from "@/api";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseCard from "@/components/base/BaseCard.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import BaseSegmentedControl from "@/components/base/BaseSegmentedControl.vue";
import BaseTextarea from "@/components/base/BaseTextarea.vue";
import BaseToast from "@/components/base/BaseToast.vue";
import {
  useLockState,
  useSecureScreen,
  useSecuritySettings,
} from "@/composables";
import { computed, onMounted, ref } from "vue";
import { useRouter } from "vue-router";

const router = useRouter();
const { onLock } = useLockState();

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

// ── App-launch biometric gate (RFC 0028) state ──────────────────────────
const appLockAvailable = ref(false);
const appLockEnabled = ref(false);
const identityAutoUnlockEnabled = ref(false);
const appLockPassphrase = ref("");
const appLockLoading = ref(false);

// ── Repository authenticity state ────────────────────────────────────────
const authConfig = ref<AuthenticityConfig | null>(null);
const authLoading = ref(false);
const showAddKey = ref(false);
const newPublicKey = ref("");
const newKeyLabel = ref("");

// ── Commit identity state ───────────────────────────────────────────────
const commitName = ref("");
const commitEmail = ref("");
const commitLoading = ref(false);
const commitDefault = ref<CommitIdentity | null>(null);

// ── Auto-lock & auto-clear state ────────────────────────────────────────
const { applySecurityConfig } = useSecuritySettings();
const { secureScreen, secureAvailable, setSecureScreen } = useSecureScreen();
const lockLoading = ref(false);

// App auto-lock presets. "Immediate" (no-cache, per-op) is the default.
const LOCK_PRESETS: { label: string; value: LockMode }[] = [
  { label: "Immediate", value: "immediate" },
  { label: "1 min", value: { idle: 60 } },
  { label: "5 min", value: { idle: 300 } },
  { label: "15 min", value: { idle: 900 } },
  { label: "30 min", value: { idle: 1800 } },
  { label: "Never", value: "never" },
];
// View-clear presets. A `null` value clears the override (tracks the default).
const VIEW_CLEAR_PRESETS: { label: string; value: number | null }[] = [
  { label: "10s", value: 10 },
  { label: "45s · default", value: null },
  { label: "3 min", value: 180 },
  { label: "Never", value: 0 },
];
// Clipboard-clear presets. Same `null` ⇒ default convention.
const CLIPBOARD_CLEAR_PRESETS: { label: string; value: number | null }[] = [
  { label: "45s · default", value: null },
  { label: "3 min", value: 180 },
  { label: "Never", value: 0 },
];

const rawLockMode = computed<LockMode>(
  () => config.value?.lock_mode ?? "immediate",
);
const rawViewClear = computed<number | null>(
  () => config.value?.view_clear_secs ?? null,
);
const rawClipboardClear = computed<number | null>(
  () => config.value?.clipboard_clear_secs ?? null,
);

// Two-arg equality for LockMode (handles the `{ idle }` object presets); passed
// to BaseSegmentedControl's `by` prop. `lockModeActive` (below) wraps it for the
// hint-line checks.
function lockModeEq(a: LockMode, b: LockMode): boolean {
  if (a === b) return true;
  if (typeof a === "object" && typeof b === "object") return a.idle === b.idle;
  return false;
}

function lockModeActive(p: LockMode): boolean {
  return lockModeEq(rawLockMode.value, p);
}

// Verification-mode pills (labels capitalize via CSS to match the prior look).
const VERIFY_MODES: {
  label: VerifyMode;
  value: VerifyMode;
  labelClass: string;
}[] = (["off", "audit", "enforce"] as VerifyMode[]).map((m) => ({
  label: m,
  value: m,
  labelClass: "capitalize",
}));

// Whether the stored identity is an SSH key. SSH keys are never
// passphrase-encrypted by gpm (they rely on their own native protection),
// so the at-rest encryption UI is hidden for them.
const isSshIdentity = computed(
  () =>
    identityType.value === "ssh_ed25519" || identityType.value === "ssh_rsa",
);

// The unlock modal keeps this page mounted on auto-lock, so wipe any in-DOM
// secret material (exported keys, typed passphrases) and close reveal panels
// the moment the identity locks.
onLock(() => {
  publicKey.value = "";
  privateKey.value = "";
  showPublic.value = false;
  showPrivate.value = false;
  showSetPassphrase.value = false;
  showChangePassphrase.value = false;
  newPassphrase.value = "";
  oldPassphrase.value = "";
  biometricPassphrase.value = "";
  appLockPassphrase.value = "";
});

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
    config.value = await getConfig();
    applySecurityConfig(config.value);
    isSsh.value = config.value.ssh_key !== null;
    const auth = await getAuthState();
    isIdentityEncrypted.value = auth.encrypted;
    identityType.value = auth.identity_type;
    biometricAvailable.value = await isBiometricAvailable();
    biometricEnabled.value = await isBiometricUnlockEnabled();
    appLockAvailable.value = await isAppLockAvailable();
    appLockEnabled.value = config.value.biometric_app_lock ?? false;
    identityAutoUnlockEnabled.value =
      config.value.unlock_identity_with_app ?? false;
    commitName.value = config.value.commit_user_name ?? "";
    commitEmail.value = config.value.commit_user_email ?? "";
    commitDefault.value = await getCommitIdentityDefault();
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Failed to load config";
  } finally {
    loading.value = false;
  }
}

async function onSaveCommitIdentity() {
  error.value = "";
  commitLoading.value = true;
  try {
    const updated = await setCommitIdentity(
      commitName.value.trim() || null,
      commitEmail.value.trim() || null,
    );
    config.value = updated;
    commitName.value = updated.commit_user_name ?? "";
    commitEmail.value = updated.commit_user_email ?? "";
    showToast("Commit identity saved");
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Failed to save commit identity";
  } finally {
    commitLoading.value = false;
  }
}

async function onSecureScreenChange(enabled: boolean) {
  const ok = await setSecureScreen(enabled);
  if (!ok) {
    showToast("Couldn't save screen-capture setting — try again");
    return;
  }
  // Disarming protection while a secret is still on screen would expose it, so
  // wipe any revealed key material first (mirrors the onLock wipe above).
  if (!enabled) {
    publicKey.value = "";
    privateKey.value = "";
    showPublic.value = false;
    showPrivate.value = false;
  }
  showToast(
    enabled
      ? "Screen capture blocked on sensitive pages"
      : "Screen capture allowed",
  );
}

async function onLockModeChange(mode: LockMode) {
  if (!config.value) return;
  lockLoading.value = true;
  error.value = "";
  try {
    const updated = await setLockMode(mode);
    config.value = updated;
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Failed to set auto-lock mode";
  } finally {
    lockLoading.value = false;
  }
}

const autosyncEnabled = computed(() => config.value?.autosync ?? true);

async function onAutosyncChange(enabled: boolean) {
  if (!config.value) return;
  lockLoading.value = true;
  error.value = "";
  try {
    const updated = await setAutosync(enabled);
    config.value = updated;
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Failed to set AutoSync";
  } finally {
    lockLoading.value = false;
  }
}

async function onViewClearChange(secs: number | null) {
  if (!config.value) return;
  lockLoading.value = true;
  error.value = "";
  try {
    const updated = await setViewClearSecs(secs);
    config.value = updated;
    applySecurityConfig(updated);
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Failed to set view auto-clear";
  } finally {
    lockLoading.value = false;
  }
}

async function onClipboardClearChange(secs: number | null) {
  if (!config.value) return;
  lockLoading.value = true;
  error.value = "";
  try {
    const updated = await setClipboardClearSecs(secs);
    config.value = updated;
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Failed to set clipboard auto-clear";
  } finally {
    lockLoading.value = false;
  }
}

async function showPublicKey() {
  error.value = "";
  try {
    const result = await getSshPublicKey();
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
    const result = await exportSshPrivateKey();
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
    await setPassphrase(newPassphrase.value);
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
    await changePassphrase(oldPassphrase.value, newPassphrase.value);
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

// ── App-launch biometric gate (RFC 0028) ─────────────────────────────────
async function onEnableAppLock() {
  error.value = "";
  appLockLoading.value = true;
  try {
    await enableBiometricAppLock();
    appLockEnabled.value = true;
    showToast("✓ App lock enabled");
  } catch (e) {
    const err = asAppLockError(e) as AppLockError;
    if (err.code === "BIOMETRIC_CANCELLED") {
      // User cancelled the migration prompt — no error toast.
    } else {
      error.value = err.message || "Failed to enable app lock";
    }
  } finally {
    appLockLoading.value = false;
  }
}

async function onDisableAppLock() {
  error.value = "";
  appLockLoading.value = true;
  try {
    await disableBiometricAppLock();
    appLockEnabled.value = false;
    // Disabling the gate makes identity auto-unlock moot.
    identityAutoUnlockEnabled.value = false;
    showToast("App lock disabled");
  } catch (e) {
    const err = asAppLockError(e) as AppLockError;
    if (err.code === "BIOMETRIC_CANCELLED") {
      // User cancelled — stays enabled.
    } else {
      error.value = err.message || "Failed to disable app lock";
    }
  } finally {
    appLockLoading.value = false;
  }
}

async function onEnableIdentityAutoUnlock() {
  error.value = "";
  if (!appLockPassphrase.value) {
    error.value = "Passphrase is required";
    return;
  }
  appLockLoading.value = true;
  try {
    await enableIdentityAutoUnlock(appLockPassphrase.value);
    identityAutoUnlockEnabled.value = true;
    appLockPassphrase.value = "";
    showToast("✓ Identity auto-unlock enabled");
  } catch (e) {
    const err = asAppLockError(e) as AppLockError;
    if (err.code === "WRONG_PASSPHRASE") {
      error.value = "Wrong passphrase";
    } else {
      error.value = err.message || "Failed to enable identity auto-unlock";
    }
  } finally {
    appLockLoading.value = false;
  }
}

async function onDisableIdentityAutoUnlock() {
  await disableIdentityAutoUnlock();
  identityAutoUnlockEnabled.value = false;
  showToast("Identity auto-unlock disabled");
}

// ── Repository authenticity ──────────────────────────────────────────────
async function loadAuthConfig() {
  try {
    authConfig.value = await getAuthenticityConfig();
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
    const effective = await setVerificationMode(mode);
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
    await addTrustedKey(key, newKeyLabel.value.trim() || "signer");
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
    await removeTrustedKey(fingerprint);
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
    await trustHeadSigner(label.trim() || "signer");
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
    await apiResetConfig();
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
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <header class="flex justify-between items-center mb-6" role="banner">
      <h1 class="text-xl">⚙ Settings</h1>
      <BaseButton size="sm" aria-label="Back to entries" @click="goBack">
        ← Back
      </BaseButton>
    </header>

    <div v-if="loading" class="text-center text-muted py-8">Loading...</div>

    <BaseAlert v-else-if="error" variant="danger" class="mb-4">
      {{ error }}
    </BaseAlert>

    <div v-else-if="config" class="flex flex-col gap-4">
      <!-- Repo info -->
      <BaseCard as="section">
        <h2 class="text-sm font-medium mb-2">Repository</h2>
        <div class="text-sm text-muted break-all">{{ config.url }}</div>
        <div class="text-xs text-subtle mt-1">
          Auth: {{ isSsh ? "SSH Key" : config.pat ? "PAT" : "None (public)" }}
        </div>
      </BaseCard>

      <!-- Commit identity -->
      <BaseCard as="section">
        <h2 class="text-sm font-medium mb-2">Commit Identity</h2>
        <p class="text-xs text-muted mb-3">
          Name and email written to each git commit. Leave blank to use the
          default<span v-if="commitDefault">
            ({{ commitDefault.name }} &lt;{{ commitDefault.email }}&gt;)</span
          >.
        </p>
        <div class="flex flex-col gap-1 mb-3">
          <label for="commit-name" class="text-xs text-muted">Name</label>
          <BaseInput
            id="commit-name"
            v-model="commitName"
            type="text"
            placeholder="Name"
            autocomplete="off"
            :disabled="commitLoading"
          />
        </div>
        <div class="flex flex-col gap-1 mb-3">
          <label for="commit-email" class="text-xs text-muted">Email</label>
          <BaseInput
            id="commit-email"
            v-model="commitEmail"
            type="email"
            placeholder="Email"
            autocomplete="off"
            :disabled="commitLoading"
          />
        </div>
        <BaseButton
          variant="action"
          :loading="commitLoading"
          @click="onSaveCommitIdentity"
        >
          Save
        </BaseButton>
      </BaseCard>

      <!-- SSH key management -->
      <BaseCard as="section" v-if="isSsh">
        <h2 class="text-sm font-medium mb-3">SSH Key</h2>

        <!-- Show public key -->
        <div class="flex flex-col gap-2">
          <BaseButton variant="action" @click="showPublicKey">
            🔑 Show Public Key
          </BaseButton>

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
          <BaseButton variant="action-danger" @click="exportPrivateKey">
            🔓 Export Private Key
          </BaseButton>

          <div v-if="showPrivate" class="mt-2 flex flex-col gap-2">
            <BaseAlert variant="danger">
              ⚠ Private key is now visible. Copy it to a safe place and close
              this screen.
            </BaseAlert>
            <div class="flex justify-end">
              <button class="btn-copy" @click="copyText(privateKey)">
                📋 Copy
              </button>
            </div>
            <pre class="key-display private-key-display">{{ privateKey }}</pre>
            <BaseButton
              variant="action"
              class="mt-1"
              @click="
                showPrivate = false;
                privateKey = '';
              "
            >
              Hide Private Key
            </BaseButton>
          </div>
        </div>
      </BaseCard>

      <!-- Passphrase management (x25519 identities only — SSH keys rely on
           their own native passphrase protection) -->
      <BaseCard as="section" v-if="!isSshIdentity">
        <h2 class="text-sm font-medium mb-3">Identity Encryption</h2>

        <!-- Not encrypted: set passphrase -->
        <template v-if="!isIdentityEncrypted">
          <p class="text-xs text-muted mb-2">
            The identity is stored in plaintext. Set a passphrase to encrypt it.
          </p>
          <BaseButton
            v-if="!showSetPassphrase"
            variant="action"
            @click="showSetPassphrase = true"
          >
            🔒 Set Passphrase
          </BaseButton>
          <div v-if="showSetPassphrase" class="flex flex-col gap-2">
            <BaseInput
              v-model="newPassphrase"
              type="password"
              placeholder="New passphrase"
              autocomplete="new-password"
              :disabled="passphraseLoading"
            />
            <BaseButton
              variant="action"
              :loading="passphraseLoading"
              @click="onSetPassphrase"
            >
              Encrypt Identity
            </BaseButton>
          </div>
        </template>

        <!-- Encrypted: change passphrase -->
        <template v-else>
          <p class="text-xs text-muted mb-2">
            ✓ Identity is passphrase-encrypted.
          </p>
          <BaseButton
            v-if="!showChangePassphrase"
            variant="action"
            @click="showChangePassphrase = true"
          >
            🔑 Change Passphrase
          </BaseButton>
          <div v-if="showChangePassphrase" class="flex flex-col gap-2">
            <BaseInput
              v-model="oldPassphrase"
              type="password"
              placeholder="Current passphrase"
              autocomplete="current-password"
              :disabled="passphraseLoading"
            />
            <BaseInput
              v-model="newPassphrase"
              type="password"
              placeholder="New passphrase"
              autocomplete="new-password"
              :disabled="passphraseLoading"
            />
            <BaseButton
              variant="action"
              :loading="passphraseLoading"
              @click="onChangePassphrase"
            >
              Change Passphrase
            </BaseButton>
          </div>
        </template>
      </BaseCard>

      <!-- SSH key identities are not encrypted by gpm -->
      <BaseCard as="section" v-else>
        <h2 class="text-sm font-medium mb-3">Identity Encryption</h2>
        <p class="text-xs text-muted">
          SSH key identities rely on their own native passphrase protection and
          are not re-encrypted by gpm.
        </p>
      </BaseCard>

      <!-- Biometric unlock (only meaningful when the identity is encrypted) -->
      <BaseCard as="section" v-if="isIdentityEncrypted">
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
            <BaseInput
              v-model="biometricPassphrase"
              type="password"
              placeholder="Current passphrase"
              autocomplete="current-password"
              :disabled="biometricLoading"
            />
            <BaseButton
              variant="action"
              :loading="biometricLoading"
              @click="onEnableBiometric"
            >
              Enable Biometric
            </BaseButton>
          </div>
        </template>

        <template v-else>
          <p class="text-xs text-muted mb-2">✓ Biometric unlock is enabled.</p>
          <BaseButton variant="action-danger" @click="onDisableBiometric">
            Disable Biometric
          </BaseButton>
        </template>
      </BaseCard>

      <!-- App-launch biometric gate (RFC 0028) -->
      <BaseCard as="section" v-if="appLockAvailable">
        <h2 class="text-sm font-medium mb-3">App Lock</h2>
        <p class="text-xs text-muted mb-3">
          Require biometric every time you open or return to gpm. When on, the
          whole store is sealed behind your fingerprint — nothing is visible
          until you authenticate.
        </p>

        <!-- App lock enable/disable -->
        <template v-if="!appLockEnabled">
          <BaseButton
            variant="action"
            :loading="appLockLoading"
            @click="onEnableAppLock"
          >
            🔒 Enable App Lock
          </BaseButton>
        </template>

        <template v-else>
          <p class="text-xs text-muted mb-2">✓ App lock is enabled.</p>
          <BaseButton
            variant="action-danger"
            :disabled="appLockLoading"
            @click="onDisableAppLock"
          >
            Disable App Lock
          </BaseButton>

          <!-- Identity auto-unlock opt-in (req3): separate from the auto-lock
               timing presets below; only meaningful with the gate on and an
               encrypted identity. -->
          <div
            v-if="isIdentityEncrypted"
            class="mt-4 pt-4 border-t border-edge"
          >
            <h3 class="text-sm font-medium mb-1">Identity Auto-Unlock</h3>
            <p class="text-xs text-muted mb-3">
              Also unlock your passwords when you unlock the app — one biometric
              prompt does both. Optional and off by default.
            </p>
            <template v-if="!identityAutoUnlockEnabled">
              <div class="flex flex-col gap-2">
                <BaseInput
                  v-model="appLockPassphrase"
                  type="password"
                  placeholder="Current passphrase"
                  autocomplete="current-password"
                  :disabled="appLockLoading"
                />
                <BaseButton
                  variant="action"
                  :loading="appLockLoading"
                  @click="onEnableIdentityAutoUnlock"
                >
                  Enable Auto-Unlock
                </BaseButton>
              </div>
            </template>
            <template v-else>
              <p class="text-xs text-muted mb-2">
                ✓ Identity unlocks together with the app.
              </p>
              <BaseButton
                variant="action-danger"
                :disabled="appLockLoading"
                @click="onDisableIdentityAutoUnlock"
              >
                Disable Auto-Unlock
              </BaseButton>
            </template>
          </div>
        </template>
      </BaseCard>

      <!-- Screen capture protection (Android FLAG_SECURE) — Android only -->
      <BaseCard as="section" v-if="secureAvailable">
        <h2 class="text-sm font-medium mb-2">Screen capture protection</h2>
        <p class="text-xs text-muted mb-3">
          Block screenshots and screen recording on pages that show secrets
          (setup, create, generate, entry, settings — including the SSH key
          export). Android only.
        </p>
        <BaseSegmentedControl
          name="secure-screen"
          legend="Block screen capture"
          :model-value="secureScreen"
          :options="[
            { label: 'On', value: true },
            { label: 'Off', value: false },
          ]"
          @change="onSecureScreenChange"
        >
          <template #hint>
            <p class="text-xs text-subtle mt-1">
              <template v-if="secureScreen"
                >Sensitive pages block capture; the entry list and history stay
                capturable.</template
              >
              <template v-else>No page blocks screen capture.</template>
            </p>
          </template>
        </BaseSegmentedControl>
      </BaseCard>

      <!-- AutoSync -->
      <BaseCard as="section" v-if="config">
        <h2 class="text-sm font-medium mb-3">AutoSync</h2>
        <BaseSegmentedControl
          class="mb-3"
          name="autosync"
          legend="Publish on every save"
          :model-value="autosyncEnabled"
          :options="[
            { label: 'On', value: true },
            { label: 'Off', value: false },
          ]"
          :disabled="lockLoading"
          @change="onAutosyncChange"
        >
          <template #hint>
            <p class="text-xs text-subtle mt-1">
              <template v-if="autosyncEnabled"
                >Each save pulls, commits, and pushes automatically.</template
              >
              <template v-else
                >Saves stay local until you Sync. Direct collisions show a
                resolve prompt; rarely, an edit from an out-of-date view can
                overwrite a newer change without a prompt — recoverable in git
                history.</template
              >
            </p>
          </template>
        </BaseSegmentedControl>
      </BaseCard>

      <!-- Auto-lock & auto-clear -->
      <BaseCard as="section" v-if="config">
        <h2 class="text-sm font-medium mb-3">Auto-Lock &amp; Auto-Clear</h2>
        <p class="text-xs text-muted mb-3">
          Control when the identity locks and how long secrets stay in the
          clipboard / on screen.
        </p>

        <!-- App auto-lock mode -->
        <BaseSegmentedControl
          class="mb-3"
          name="lock-mode"
          legend="Auto-lock"
          wrap
          :model-value="rawLockMode"
          :by="lockModeEq"
          :options="LOCK_PRESETS"
          :disabled="lockLoading"
          @change="onLockModeChange"
        >
          <template #hint>
            <p class="text-xs text-subtle mt-1">
              <template v-if="lockModeActive('immediate')"
                >Per-operation: re-authenticate each copy/show/create. Strongest
                default.</template
              >
              <template v-else-if="lockModeActive('never')"
                >Never auto-lock; the identity stays cached until you lock
                manually.</template
              >
              <template v-else
                >Session: stay unlocked, lock after the idle period.</template
              >
            </p>
          </template>
        </BaseSegmentedControl>

        <!-- View auto-clear -->
        <BaseSegmentedControl
          class="mb-3"
          name="view-clear"
          legend="Password view auto-clear"
          wrap
          :model-value="rawViewClear"
          :options="VIEW_CLEAR_PRESETS"
          :disabled="lockLoading"
          @change="onViewClearChange"
        />

        <!-- Clipboard auto-clear -->
        <BaseSegmentedControl
          name="clipboard-clear"
          legend="Clipboard auto-clear"
          wrap
          :model-value="rawClipboardClear"
          :options="CLIPBOARD_CLEAR_PRESETS"
          :disabled="lockLoading"
          @change="onClipboardClearChange"
        />
      </BaseCard>

      <!-- Repository authenticity -->
      <BaseCard as="section" v-if="authConfig">
        <h2 class="text-sm font-medium mb-3">Repository Authenticity</h2>
        <p class="text-xs text-muted mb-3">
          Verify SSH-signed commits on every pull to detect a compromised remote
          feeding validly encrypted but wrong entries.
        </p>

        <!-- Mode selector -->
        <BaseSegmentedControl
          class="mb-3"
          name="verify-mode"
          legend="Verification"
          :model-value="authConfig.mode"
          :options="VERIFY_MODES"
          :disabled="authLoading"
          @change="onModeChange"
        >
          <template #hint>
            <p class="text-xs text-subtle mt-1">
              <template v-if="authConfig.mode === 'off'"
                >No verification.</template
              >
              <template v-else-if="authConfig.mode === 'audit'"
                >Verify and warn, but always pull.</template
              >
              <template v-else>Block pulls with unverified commits.</template>
            </p>
          </template>
        </BaseSegmentedControl>

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
          <BaseButton
            v-if="authConfig.trusted_keys.length === 0"
            variant="action"
            @click="onTrustHead"
          >
            🔑 Trust this repo's signer (HEAD)
          </BaseButton>
          <BaseButton
            v-if="!showAddKey"
            variant="action"
            @click="showAddKey = true"
          >
            + Add a signing public key
          </BaseButton>
          <div v-if="showAddKey" class="flex flex-col gap-2">
            <BaseTextarea
              v-model="newPublicKey"
              rows="2"
              placeholder="ssh-ed25519 AAAA… [comment]"
              class="font-mono text-xs"
            />
            <BaseInput
              v-model="newKeyLabel"
              type="text"
              placeholder="Label (e.g. Alice — laptop)"
            />
            <div class="flex gap-2">
              <BaseButton
                variant="action"
                class="flex-1"
                :disabled="authLoading"
                @click="onAddKey"
              >
                Save key
              </BaseButton>
              <BaseButton
                variant="action"
                class="flex-1"
                @click="
                  showAddKey = false;
                  newPublicKey = '';
                  newKeyLabel = '';
                "
              >
                Cancel
              </BaseButton>
            </div>
          </div>
          <BaseButton variant="action" @click="openHistory">
            📜 View commit history &amp; signatures
          </BaseButton>
        </div>
      </BaseCard>

      <!-- Danger zone -->
      <BaseCard as="section" border="danger">
        <h2 class="text-sm font-medium mb-2 text-danger">Danger Zone</h2>
        <BaseButton variant="action-danger" @click="resetConfig">
          🗑 Reset All Data
        </BaseButton>
        <p class="text-xs text-subtle mt-1">
          Remove all local data and configuration.
        </p>
      </BaseCard>
    </div>

    <!-- Toast -->
    <BaseToast v-if="toast" variant="success">{{ toast }}</BaseToast>
  </main>
</template>

<style scoped>
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
