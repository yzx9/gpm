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
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import BaseModalShell from "@/components/base/BaseModalShell.vue";
import BaseSegmentedControl from "@/components/base/BaseSegmentedControl.vue";
import BaseTextarea from "@/components/base/BaseTextarea.vue";
import PassphraseField from "@/components/PassphraseField.vue";
import {
  useLockState,
  useOverlayBackHandler,
  useSecureScreen,
  useSecuritySettings,
  useToast,
} from "@/composables";
import {
  ArrowLeft,
  CircleCheck,
  Copy,
  History,
  KeyRound,
  Lock,
  LockOpen,
  Plus,
  Settings,
  Trash2,
  TriangleAlert,
} from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { onBeforeRouteLeave, useRouter } from "vue-router";

const router = useRouter();
const { onLock } = useLockState();
const { toast } = useToast();

const config = ref<RepoConfig | null>(null);
const loading = ref(false);
const error = ref("");
const publicKey = ref("");
const showPublic = ref(false);
const privateKey = ref("");
const showPrivate = ref(false);

const isSsh = ref(false);

// ── Passphrase management state ──────────────────────────────────────────
const isIdentityEncrypted = ref(false);
const identityType = ref("");

// Shared passphrase modal — one prompt for set / change / enable-biometric /
// enable-auto-unlock. The modal is the commit boundary: submit saves+closes,
// cancel / backdrop / Android-back wipes the inputs and closes.
type PassphraseMode =
  | "set"
  | "change"
  | "enable-biometric"
  | "enable-auto-unlock";
const passphraseModal = ref<PassphraseMode | null>(null);
const ppCurrent = ref("");
const ppNew = ref("");
const passphraseLoading = ref(false);
// PassphraseField instance for the modal's set/change new-passphrase (gives
// the confirm box + validate() so setting a passphrase asks you to type it
// twice and checks the two match before submitting).
const ppField = ref<InstanceType<typeof PassphraseField> | null>(null);

const ppModalTitle = computed(() => {
  switch (passphraseModal.value) {
    case "set":
      return "Set Passphrase";
    case "change":
      return "Change Passphrase";
    case "enable-biometric":
      return "Enable Biometric Unlock";
    case "enable-auto-unlock":
      return "Enable Identity Auto-Unlock";
    default:
      return "";
  }
});
const ppSubmitLabel = computed(() => {
  switch (passphraseModal.value) {
    case "set":
      return "Encrypt Identity";
    case "change":
      return "Change Passphrase";
    case "enable-biometric":
      return "Enable Biometric";
    case "enable-auto-unlock":
      return "Enable";
    default:
      return "";
  }
});
const ppShowCurrent = computed(
  () =>
    passphraseModal.value === "change" ||
    passphraseModal.value === "enable-biometric" ||
    passphraseModal.value === "enable-auto-unlock",
);
const ppShowNew = computed(
  () => passphraseModal.value === "set" || passphraseModal.value === "change",
);

// ── Biometric unlock state ──────────────────────────────────────────────
const biometricAvailable = ref(false);
const biometricEnabled = ref(false);
const biometricLoading = ref(false);

// ── App-launch biometric gate (RFC 0028) state ──────────────────────────
const appLockAvailable = ref(false);
const appLockEnabled = ref(false);
const identityAutoUnlockEnabled = ref(false);
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
  // Wipe any in-flight passphrase-modal input as defense-in-depth; the modal
  // sits below the lock overlay and should not retain a typed secret.
  ppCurrent.value = "";
  ppNew.value = "";
  passphraseModal.value = null;
});

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

async function onSaveCommitIdentity(): Promise<boolean> {
  error.value = "";
  commitLoading.value = true;
  try {
    const updated = await setCommitIdentity(
      commitName.value.trim() || null,
      commitEmail.value.trim() || null,
    );
    config.value = updated;
    // Re-sync from the response (trimmed) so a successful Save clears dirty.
    commitName.value = updated.commit_user_name ?? "";
    commitEmail.value = updated.commit_user_email ?? "";
    toast.success("Commit identity saved");
    return true;
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Failed to save commit identity";
    return false;
  } finally {
    commitLoading.value = false;
  }
}

async function onSecureScreenChange(enabled: boolean) {
  const ok = await setSecureScreen(enabled);
  if (!ok) {
    toast.danger("Couldn't save screen-capture setting — try again");
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
  toast.success(
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
    toast.success("✓ Copied to clipboard");
  } catch {
    toast.danger("Copy failed");
  }
}

function openPassphraseModal(mode: PassphraseMode) {
  ppCurrent.value = "";
  ppNew.value = "";
  error.value = "";
  passphraseModal.value = mode;
}

function closePassphraseModal() {
  ppCurrent.value = "";
  ppNew.value = "";
  passphraseModal.value = null;
}

// One commit boundary for every passphrase operation. Submit dispatches to
// the relevant API with the mode's error mapping; success wipes + closes,
// failure keeps the modal open so the user can correct and retry.
async function onPassphraseSubmit() {
  const mode = passphraseModal.value;
  if (!mode) return;
  error.value = "";
  if (mode === "change" && !ppCurrent.value) {
    error.value = "Current passphrase is required";
    return;
  }
  if (ppShowCurrent.value && !ppCurrent.value) {
    error.value = "Passphrase is required";
    return;
  }
  // set / change enter the new passphrase via PassphraseField (with a confirm
  // box); validate the two match before dispatching.
  if (ppShowNew.value) {
    const passphraseError = ppField.value?.validate() ?? null;
    if (passphraseError) {
      error.value = passphraseError;
      return;
    }
  }
  passphraseLoading.value = true;
  try {
    if (mode === "set") {
      await setPassphrase(ppNew.value);
      isIdentityEncrypted.value = true;
      // Setting a passphrase can invalidate a previously-sealed biometric unlock.
      biometricEnabled.value = await isBiometricUnlockEnabled();
      toast.success("✓ Passphrase set — identity is now encrypted");
    } else if (mode === "change") {
      await changePassphrase(ppCurrent.value, ppNew.value);
      biometricEnabled.value = await isBiometricUnlockEnabled();
      toast.success("✓ Passphrase changed");
    } else if (mode === "enable-biometric") {
      await enableBiometricUnlock(ppCurrent.value);
      biometricEnabled.value = true;
      toast.success("✓ Biometric unlock enabled");
    } else {
      await enableIdentityAutoUnlock(ppCurrent.value);
      identityAutoUnlockEnabled.value = true;
      toast.success("✓ Identity auto-unlock enabled");
    }
    closePassphraseModal();
  } catch (e) {
    if (mode === "enable-biometric") {
      const err = e as BiometricError;
      if (err.code === "BIOMETRIC_CANCELLED") {
        // User cancelled the biometric prompt — keep the modal open for retry.
      } else if (err.code === "WRONG_PASSPHRASE") {
        error.value = "Wrong passphrase";
      } else {
        error.value = err.message || "Failed to enable biometric";
      }
    } else if (mode === "enable-auto-unlock") {
      const err = asAppLockError(e) as AppLockError;
      error.value =
        err.code === "WRONG_PASSPHRASE"
          ? "Wrong passphrase"
          : err.message || "Failed to enable identity auto-unlock";
    } else {
      const appError = e as AppError;
      error.value =
        appError?.message ||
        (mode === "set"
          ? "Failed to set passphrase"
          : "Failed to change passphrase");
    }
  } finally {
    passphraseLoading.value = false;
  }
}

async function onDisableBiometric() {
  await disableBiometricUnlock();
  biometricEnabled.value = false;
  toast.success("Biometric unlock disabled");
}

// ── App-launch biometric gate (RFC 0028) ─────────────────────────────────
async function onEnableAppLock() {
  error.value = "";
  appLockLoading.value = true;
  try {
    await enableBiometricAppLock();
    appLockEnabled.value = true;
    toast.success("✓ App lock enabled");
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
    toast.success("App lock disabled");
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

async function onDisableIdentityAutoUnlock() {
  await disableIdentityAutoUnlock();
  identityAutoUnlockEnabled.value = false;
  toast.success("Identity auto-unlock disabled");
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

async function onAddKey(): Promise<boolean> {
  error.value = "";
  const key = newPublicKey.value.trim();
  if (!key) {
    error.value = "Paste an SSH signing public key";
    return false;
  }
  authLoading.value = true;
  try {
    await addTrustedKey(key, newKeyLabel.value.trim() || "signer");
    newPublicKey.value = "";
    newKeyLabel.value = "";
    showAddKey.value = false;
    toast.success("✓ Trusted signing key added");
    await loadAuthConfig();
    return true;
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Failed to add key";
    return false;
  } finally {
    authLoading.value = false;
  }
}

async function onRemoveKey(fingerprint: string) {
  if (!confirm("Remove this trusted signing key?")) return;
  authLoading.value = true;
  try {
    await removeTrustedKey(fingerprint);
    toast.success("Trusted key removed");
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
    toast.success("✓ HEAD signer trusted");
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

// Reset is gated behind a type-"RESET"-to-confirm modal: a stray tap can't
// trigger this unrecoverable wipe, and no passphrase is required, so a user
// who forgot theirs can still reset. (The lock dialogs no longer offer reset.)
const RESET_CONFIRM_WORD = "RESET";
const resetOpen = ref(false);
const resetConfirmText = ref("");
const resetReady = computed(
  () => resetConfirmText.value.trim().toUpperCase() === RESET_CONFIRM_WORD,
);

function resetConfig() {
  resetConfirmText.value = "";
  resetOpen.value = true;
}

async function doReset() {
  if (!resetReady.value) return;
  try {
    await apiResetConfig();
    resetOpen.value = false;
    router.push({ name: "setup" });
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Reset failed";
    resetOpen.value = false;
  }
}

function goBack() {
  router.push({ name: "entries" });
}

// ── Deferred-save dirty tracking + leave guard (Commit Identity, Add Key) ──
// These two text forms stage edits in refs and only commit on their own Save
// button. Leaving the page with uncommitted edits prompts Discard / Save /
// Keep editing, so a stray back-tap never silently throws away typed input.
const commitDirty = computed(() => {
  const name = config.value?.commit_user_name ?? "";
  const email = config.value?.commit_user_email ?? "";
  return commitName.value.trim() !== name || commitEmail.value.trim() !== email;
});
const addKeyDirty = computed(
  () =>
    showAddKey.value &&
    (newPublicKey.value.trim() !== "" || newKeyLabel.value.trim() !== ""),
);
const hasUnsavedChanges = computed(
  () => commitDirty.value || addKeyDirty.value,
);

// Commit every dirty form. Returns false on any failure so the leave guard
// can keep the user on the page to see the error; each handler already sets
// `error` and leaves the form dirty on failure.
async function commitAllPending(): Promise<boolean> {
  let ok = true;
  if (commitDirty.value) ok = (await onSaveCommitIdentity()) && ok;
  if (addKeyDirty.value) ok = (await onAddKey()) && ok;
  return ok;
}

type UnsavedChoice = "discard" | "save" | "cancel";
const unsavedOpen = ref(false);
let pendingResolve: ((c: UnsavedChoice) => void) | null = null;
// Re-entrancy guard: if a second navigation supersedes the guarded one while
// the modal is open, only the latest token's resolver runs; older ones no-op.
let promptToken = 0;

function promptUnsaved(): Promise<UnsavedChoice> {
  const token = ++promptToken;
  unsavedOpen.value = true;
  return new Promise<UnsavedChoice>((resolve) => {
    pendingResolve = (c) => {
      if (token === promptToken) resolve(c);
    };
  });
}

function resolveUnsaved(c: UnsavedChoice) {
  const r = pendingResolve;
  pendingResolve = null;
  unsavedOpen.value = false;
  r?.(c);
}

onBeforeRouteLeave(async () => {
  if (!hasUnsavedChanges.value) return true;
  const choice = await promptUnsaved();
  if (choice === "cancel") return false;
  if (choice === "save") {
    const ok = await commitAllPending();
    if (!ok) return false; // keep the user on the page so the error is visible
  }
  return true; // discard, or save succeeded
});

// Android back inside either modal dismisses it (= cancel), mirroring the
// UnlockModal / AppLockOverlay pattern. Both register concurrently; only one
// is shown at a time, so two idle listeners are benign.
useOverlayBackHandler(
  computed(() => passphraseModal.value !== null),
  () => closePassphraseModal(),
);
useOverlayBackHandler(
  computed(() => unsavedOpen.value),
  () => resolveUnsaved("cancel"),
);

onMounted(() => {
  loadConfig();
  loadAuthConfig();
});
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <header class="flex justify-between items-center mb-6" role="banner">
      <h1 class="text-xl flex items-center gap-1">
        <BaseIcon :icon="Settings" :size="24" /> Settings
      </h1>
      <BaseButton size="sm" aria-label="Back to entries" @click="goBack">
        <BaseIcon :icon="ArrowLeft" /> Back
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
      <BaseCard as="section" :border="commitDirty ? 'accent' : 'edge'">
        <h2 class="text-sm font-medium mb-2">
          Commit Identity
          <span
            v-if="commitDirty"
            class="ml-1 text-xs font-normal"
            style="color: var(--color-accent)"
            >Unsaved changes</span
          >
        </h2>
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
            <BaseIcon :icon="KeyRound" /> Show Public Key
          </BaseButton>

          <div v-if="showPublic" class="mt-2 flex flex-col gap-2">
            <div class="flex justify-between items-center">
              <span class="text-xs text-muted">Public key</span>
              <button class="btn-copy" @click="copyText(publicKey)">
                <BaseIcon :icon="Copy" /> Copy
              </button>
            </div>
            <pre class="key-display">{{ publicKey }}</pre>
          </div>
        </div>

        <!-- Export private key -->
        <div class="flex flex-col gap-2 mt-3">
          <BaseButton variant="action-danger" @click="exportPrivateKey">
            <BaseIcon :icon="LockOpen" /> Export Private Key
          </BaseButton>

          <div v-if="showPrivate" class="mt-2 flex flex-col gap-2">
            <BaseAlert variant="danger">
              <BaseIcon
                :icon="TriangleAlert"
                :size="14"
                class="inline-block align-middle"
              />
              Private key is now visible. Copy it to a safe place and close this
              screen.
            </BaseAlert>
            <div class="flex justify-end">
              <button class="btn-copy" @click="copyText(privateKey)">
                <BaseIcon :icon="Copy" /> Copy
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
           their own native passphrase protection). Set / change run in the
           shared passphrase modal, which is the commit boundary. -->
      <BaseCard as="section" v-if="!isSshIdentity">
        <h2 class="text-sm font-medium mb-3">Identity Encryption</h2>

        <!-- Not encrypted: set passphrase -->
        <template v-if="!isIdentityEncrypted">
          <p class="text-xs text-muted mb-2">
            The identity is stored in plaintext. Set a passphrase to encrypt it.
          </p>
          <BaseButton variant="action" @click="openPassphraseModal('set')">
            <BaseIcon :icon="Lock" /> Set Passphrase
          </BaseButton>
        </template>

        <!-- Encrypted: change passphrase -->
        <template v-else>
          <p class="text-xs text-muted mb-2 flex items-center gap-1">
            <BaseIcon :icon="CircleCheck" :size="14" class="text-success" />
            Identity is passphrase-encrypted.
          </p>
          <BaseButton variant="action" @click="openPassphraseModal('change')">
            <BaseIcon :icon="KeyRound" /> Change Passphrase
          </BaseButton>
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
          <BaseButton
            variant="action"
            :disabled="biometricLoading"
            @click="openPassphraseModal('enable-biometric')"
          >
            Enable Biometric
          </BaseButton>
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
            <BaseIcon :icon="Lock" /> Enable App Lock
          </BaseButton>
        </template>

        <template v-else>
          <p class="text-xs text-muted mb-2 flex items-center gap-1">
            <BaseIcon :icon="CircleCheck" :size="14" class="text-success" />
            App lock is enabled.
          </p>
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
              <BaseButton
                variant="action"
                :disabled="appLockLoading"
                @click="openPassphraseModal('enable-auto-unlock')"
              >
                Enable Auto-Unlock
              </BaseButton>
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
      <BaseCard
        as="section"
        v-if="authConfig"
        :border="addKeyDirty ? 'accent' : 'edge'"
      >
        <h2 class="text-sm font-medium mb-3">
          Repository Authenticity
          <span
            v-if="addKeyDirty"
            class="ml-1 text-xs font-normal"
            style="color: var(--color-accent)"
            >Unsaved changes</span
          >
        </h2>
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
            <BaseIcon :icon="KeyRound" /> Trust this repo's signer (HEAD)
          </BaseButton>
          <BaseButton
            v-if="!showAddKey"
            variant="action"
            @click="showAddKey = true"
          >
            <BaseIcon :icon="Plus" /> Add a signing public key
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
            <BaseIcon :icon="History" /> View commit history &amp; signatures
          </BaseButton>
        </div>
      </BaseCard>

      <!-- Danger zone -->
      <BaseCard as="section" border="danger">
        <h2 class="text-sm font-medium mb-2 text-danger">Danger Zone</h2>
        <BaseButton variant="action-danger" @click="resetConfig">
          <BaseIcon :icon="Trash2" /> Reset All Data
        </BaseButton>
        <p class="text-xs text-subtle mt-1">
          Remove all local data and configuration.
        </p>
      </BaseCard>
    </div>

    <!-- Reset confirmation: type RESET to confirm (z=80 stacks above UnlockModal). -->
    <BaseModalShell
      v-if="resetOpen"
      variant="center"
      :z="80"
      role="alertdialog"
      aria-label="Reset all data"
      @close="resetOpen = false"
    >
      <h2 class="text-lg font-medium text-danger mb-3">Reset all data?</h2>
      <BaseAlert variant="danger" class="mb-4">
        This permanently removes every secret, the signing identity, and all
        configuration from this device. Your passphrase cannot recover a reset
        store — you would need to set gpm up again.
      </BaseAlert>
      <div class="flex flex-col gap-1 mb-4">
        <label class="text-sm font-medium" for="reset-confirm"
          >Type RESET to confirm</label
        >
        <BaseInput
          id="reset-confirm"
          v-model="resetConfirmText"
          autocomplete="off"
          autofocus
        />
      </div>
      <div class="flex gap-2 justify-end">
        <BaseButton variant="secondary" @click="resetOpen = false"
          >Cancel</BaseButton
        >
        <BaseButton variant="danger" :disabled="!resetReady" @click="doReset">
          <BaseIcon :icon="Trash2" /> Reset all data
        </BaseButton>
      </div>
    </BaseModalShell>

    <!-- Shared passphrase modal (set / change / enable-biometric /
         enable-auto-unlock). z=50 sits below the z=60/70 lock overlays so an
         auto-lock while it's open stacks UnlockModal / AppLockOverlay above. -->
    <BaseModalShell
      v-if="passphraseModal"
      variant="sheet"
      :z="50"
      role="dialog"
      :aria-label="ppModalTitle"
      @close="closePassphraseModal"
    >
      <h2 class="text-lg font-medium mb-3">{{ ppModalTitle }}</h2>
      <div v-if="ppShowCurrent" class="flex flex-col gap-1 mb-3">
        <label for="pp-current" class="text-xs text-muted"
          >Current passphrase</label
        >
        <BaseInput
          id="pp-current"
          v-model="ppCurrent"
          type="password"
          autocomplete="current-password"
          :disabled="passphraseLoading"
        />
      </div>
      <PassphraseField
        v-if="ppShowNew"
        ref="ppField"
        id="pp-new"
        v-model="ppNew"
        :label="passphraseModal === 'change' ? 'New passphrase' : 'Passphrase'"
        placeholder="New passphrase"
        :optional="false"
        :disabled="passphraseLoading"
        class="mb-3"
      />
      <div class="flex gap-2 justify-end">
        <BaseButton
          variant="secondary"
          :disabled="passphraseLoading"
          @click="closePassphraseModal"
          >Cancel</BaseButton
        >
        <BaseButton
          variant="action"
          :loading="passphraseLoading"
          @click="onPassphraseSubmit"
          >{{ ppSubmitLabel }}</BaseButton
        >
      </div>
    </BaseModalShell>

    <!-- Unsaved-changes leave guard. Same z=50 layering as the passphrase
         modal; backdrop or Android-back = Keep editing. -->
    <BaseModalShell
      v-if="unsavedOpen"
      variant="sheet"
      :z="50"
      role="alertdialog"
      aria-label="Unsaved changes"
      @close="resolveUnsaved('cancel')"
    >
      <h2 class="text-lg font-medium mb-2">Unsaved changes</h2>
      <p class="text-sm text-muted mb-4">
        You have edits that haven't been saved. Save them before leaving, or
        discard and leave.
      </p>
      <div class="flex flex-col gap-2">
        <BaseButton variant="action" @click="resolveUnsaved('save')"
          >Save and leave</BaseButton
        >
        <BaseButton variant="secondary" @click="resolveUnsaved('cancel')"
          >Keep editing</BaseButton
        >
        <BaseButton variant="action-danger" @click="resolveUnsaved('discard')"
          >Discard and leave</BaseButton
        >
      </div>
    </BaseModalShell>
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
