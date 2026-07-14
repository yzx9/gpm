<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import type {
  AppConfig,
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
  resetConfig as apiResetConfig,
  asAppLockError,
  changePassphrase,
  disableBiometricAppLock,
  disableBiometricUnlock,
  disableIdentityAutoUnlock,
  enableBiometricAppLock,
  enableBiometricUnlock,
  enableIdentityAutoUnlock,
  getAppConfig,
  getAppLockState,
  getAuthState,
  getAuthenticityConfig,
  getCommitIdentityDefault,
  getConfig,
  getGpgKeyParseWarnings,
  importTrustedGpgKeyFile,
  isAppLockAvailable,
  isBiometricAvailable,
  isBiometricUnlockEnabled,
  removeTrustedGpgKey,
  removeTrustedKey,
  resolvedLocale,
  setAutosync,
  setClipboardClearSecs,
  setCommitIdentity,
  setLocalePref,
  setLockMode,
  setPassphrase,
  setVerificationMode,
  setViewClearSecs,
  trustHeadSigner,
} from "@/api";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseCard from "@/components/base/BaseCard.vue";
import BaseHeader from "@/components/base/BaseHeader.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import BaseModalShell from "@/components/base/BaseModalShell.vue";
import BaseSegmentedControl from "@/components/base/BaseSegmentedControl.vue";
import PassphraseField from "@/components/PassphraseField.vue";
import PassphraseUnrecoverableAck from "@/components/PassphraseUnrecoverableAck.vue";
import {
  useSecureScreen,
  useSecuritySettings,
  useToast,
  useWipeOnLeave,
} from "@/composables";
import { normalizeSupported, setLocale } from "@/i18n";
import {
  appLockEnrollPrompt,
  appLockUnlockPrompt,
  identityEnrollPrompt,
} from "@/i18n/native";
import {
  CircleCheck,
  FileUp,
  History,
  KeyRound,
  Lock,
  Plus,
  Settings,
  Trash2,
} from "@lucide/vue";
import { computed, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { onBeforeRouteLeave, useRouter } from "vue-router";

const router = useRouter();
const { toast } = useToast();
const { t } = useI18n();

// Display-language preference: "system" (track the device locale) or a pinned
// locale. Loaded from app.json on mount; the picker applies it live.
const localeSelection = ref<"system" | "en" | "zh-CN">("system");

async function loadLocalePref(): Promise<void> {
  try {
    const app = await getAppConfig();
    localeSelection.value =
      app.locale === "en" || app.locale === "zh-CN" ? app.locale : "system";
  } catch {
    // No app config yet — leave "system".
  }
}

async function onLocaleChange(selection: string): Promise<void> {
  const prev = localeSelection.value;
  try {
    // Apply the locale in-memory first and persist only on success, so a
    // failure can't leave app.json pinned to a locale the picker reverted to.
    if (selection === "system") {
      // "Track system" resolves through the backend, which normalizes the
      // device locale — apply that immediately so the switch is visible.
      await setLocale(normalizeSupported(await resolvedLocale()));
      await setLocalePref(null);
    } else if (selection === "en" || selection === "zh-CN") {
      await setLocale(selection);
      await setLocalePref(selection);
    } else {
      return; // unknown option — ignore
    }
    localeSelection.value = selection as "system" | "en" | "zh-CN";
    toast.success(t("settings.language.applied"));
  } catch {
    localeSelection.value = prev; // roll back the picker on failure
    toast.danger(t("settings.language.failed"));
  }
}

const config = ref<RepoConfig | null>(null);
const appConfig = ref<AppConfig | null>(null);
const loading = ref(false);
const error = ref("");

const isSsh = ref(false);

// ── Passphrase management state ──────────────────────────────────────────
const isIdentityEncrypted = ref(false);
const identityType = ref("");

// Shared passphrase modal — one prompt for set / change / enable-biometric /
// enable-auto-unlock. The modal is the commit boundary: submit saves+closes,
// cancel / backdrop / Android-back wipes the inputs and closes.
// prettier-ignore
type PassphraseMode =
  "set" | "change" | "enable-biometric" | "enable-auto-unlock";
const passphraseModal = ref<PassphraseMode | null>(null);
const ppCurrent = ref("");
const ppNew = ref("");
const passphraseLoading = ref(false);
// PassphraseField instance for the modal's set/change new-passphrase (gives
// the confirm box + validate() so setting a passphrase asks you to type it
// twice and checks the two match before submitting).
const ppField = ref<InstanceType<typeof PassphraseField> | null>(null);
// Forced "this passphrase cannot be recovered" acknowledgment for set/change.
// Reset on every modal open/close so an old ack can't carry across sessions.
const ppAck = ref(false);

const ppModalTitle = computed(() => {
  switch (passphraseModal.value) {
    case "set":
      return t("settings.passphrase.modal.set.title");
    case "change":
      return t("settings.passphrase.modal.change.title");
    case "enable-biometric":
      return t("settings.passphrase.modal.enableBiometric.title");
    case "enable-auto-unlock":
      return t("settings.passphrase.modal.enableAutoUnlock.title");
    default:
      return "";
  }
});
const ppSubmitLabel = computed(() => {
  switch (passphraseModal.value) {
    case "set":
      return t("settings.passphrase.modal.set.submit");
    case "change":
      return t("settings.passphrase.modal.change.submit");
    case "enable-biometric":
      return t("settings.passphrase.modal.enableBiometric.submit");
    case "enable-auto-unlock":
      return t("settings.passphrase.modal.enableAutoUnlock.submit");
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
// Submit is blocked until the user acknowledges a NEW passphrase (set/change)
// is unrecoverable. Empty passphrase is rejected by validate() on submit, so
// the gate only needs to engage once something has been typed.
const ppSubmitDisabled = computed(
  () => ppShowNew.value && !!ppNew.value && !ppAck.value,
);
// Invalidate the ack whenever the typed passphrase changes — each distinct
// committed value gets its own acknowledgment (ack is value-bound, not
// modal-bound), so editing the passphrase after ticking forces a re-ack.
watch(ppNew, () => {
  ppAck.value = false;
});

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
/** Per-key parse warnings for the persisted trusted GPG keys — non-empty when
 * a previously-trusted key later fails to re-parse (surfaces the silent
 * Verified→UnverifiedSignature downgrade so the user can re-add or remove). */
const gpgWarnings = ref<string[]>([]);

/** SSH + GPG trusted keys flattened into render rows tagged by origin list, so
 * the combined list can badge GPG entries and route each Remove to the right
 * command without sniffing the fingerprint string. */
const trustedKeyRows = computed<
  ReadonlyArray<{ kind: "ssh" | "gpg"; fingerprint: string; label: string }>
>(() => {
  const cfg = authConfig.value;
  if (!cfg) return [];
  return [
    ...cfg.trusted_keys.map((k) => ({
      kind: "ssh" as const,
      fingerprint: k.fingerprint,
      label: k.label,
    })),
    ...cfg.trusted_gpg_keys.map((k) => ({
      kind: "gpg" as const,
      fingerprint: k.fingerprint,
      label: k.label,
    })),
  ];
});

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
// Computed (not a plain const) so the labels can resolve through t().
const LOCK_PRESETS = computed<{ label: string; value: LockMode }[]>(() => [
  { label: t("settings.lock.immediate"), value: "immediate" },
  { label: t("settings.lock.minutes", { count: 1 }), value: { idle: 60 } },
  { label: t("settings.lock.minutes", { count: 5 }), value: { idle: 300 } },
  { label: t("settings.lock.minutes", { count: 15 }), value: { idle: 900 } },
  { label: t("settings.lock.minutes", { count: 30 }), value: { idle: 1800 } },
  { label: t("settings.lock.never"), value: "never" },
]);
// View-clear presets. A `null` value clears the override (tracks the default).
const VIEW_CLEAR_PRESETS = computed<{ label: string; value: number | null }[]>(
  () => [
    { label: t("settings.clear.seconds", { count: 10 }), value: 10 },
    { label: t("settings.clear.default", { count: 45 }), value: null },
    { label: t("settings.lock.minutes", { count: 3 }), value: 180 },
    { label: t("settings.lock.never"), value: 0 },
  ],
);
// Clipboard-clear presets. Same `null` ⇒ default convention.
const CLIPBOARD_CLEAR_PRESETS = computed<
  { label: string; value: number | null }[]
>(() => [
  { label: t("settings.clear.default", { count: 45 }), value: null },
  { label: t("settings.lock.minutes", { count: 3 }), value: 180 },
  { label: t("settings.lock.never"), value: 0 },
]);

const rawLockMode = computed<LockMode>(
  () => appConfig.value?.lock_mode ?? "immediate",
);
const rawViewClear = computed<number | null>(
  () => appConfig.value?.view_clear_secs ?? null,
);
const rawClipboardClear = computed<number | null>(
  () => appConfig.value?.clipboard_clear_secs ?? null,
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
// so the seal encryption UI is hidden for them.
const isSshIdentity = computed(
  () =>
    identityType.value === "ssh_ed25519" || identityType.value === "ssh_rsa",
);

/** Wipe every in-DOM secret: the typed passphrase-modal inputs and their
 *  confirm echo. Idempotent — fires on a hard lock, browser back, and unmount.
 *  (Exported SSH keys now live on the dedicated SshKeyPage, which wipes itself.) */
function wipeSecrets() {
  ppCurrent.value = "";
  ppNew.value = "";
  ppAck.value = false;
  passphraseModal.value = null;
  ppField.value?.reset();
}

// The unlock modal keeps this page mounted on auto-lock, so unmount alone can't
// guarantee a wipe — clear on a hard lock, on browser back, and on unmount.
useWipeOnLeave(wipeSecrets);

async function loadConfig() {
  loading.value = true;
  error.value = "";
  try {
    config.value = await getConfig();
    appConfig.value = await getAppConfig();
    applySecurityConfig(appConfig.value);
    isSsh.value = config.value.ssh_key !== null;
    const auth = await getAuthState();
    isIdentityEncrypted.value = auth.encrypted;
    identityType.value = auth.identity_type;
    biometricAvailable.value = await isBiometricAvailable();
    biometricEnabled.value = await isBiometricUnlockEnabled();
    appLockAvailable.value = await isAppLockAvailable();
    // The app-lock toggle reads Keystore truth (Path B), not the persisted
    // config flag — the two can drift, and the runtime gate is what matters.
    appLockEnabled.value = (await getAppLockState()).enabled;
    identityAutoUnlockEnabled.value =
      config.value.unlock_identity_with_app ?? false;
    commitName.value = config.value.commit_user_name ?? "";
    commitEmail.value = config.value.commit_user_email ?? "";
    commitDefault.value = await getCommitIdentityDefault();
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("settings.commit.loadFailed");
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
    toast.success(t("settings.commit.saved"));
    return true;
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("settings.commit.saveFailed");
    return false;
  } finally {
    commitLoading.value = false;
  }
}

async function onSecureScreenChange(enabled: boolean) {
  const ok = await setSecureScreen(enabled);
  if (!ok) {
    toast.danger(t("settings.secureScreen.saveFailed"));
    return;
  }
  toast.success(
    enabled
      ? t("settings.secureScreen.blockedToast")
      : t("settings.secureScreen.allowedToast"),
  );
}

async function onLockModeChange(mode: LockMode) {
  if (!appConfig.value) return;
  lockLoading.value = true;
  error.value = "";
  try {
    appConfig.value = await setLockMode(mode);
    // Keep the reactive lockMode ref in sync so the activity bumper's filter
    // picks up the new mode immediately (mirrors onViewClearChange below).
    applySecurityConfig(appConfig.value);
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("settings.lock.setModeFailed");
  } finally {
    lockLoading.value = false;
  }
}

const autosyncEnabled = computed(() => appConfig.value?.autosync ?? true);

async function onAutosyncChange(enabled: boolean) {
  if (!appConfig.value) return;
  lockLoading.value = true;
  error.value = "";
  try {
    appConfig.value = await setAutosync(enabled);
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("settings.autosync.setFailed");
  } finally {
    lockLoading.value = false;
  }
}

async function onViewClearChange(secs: number | null) {
  if (!appConfig.value) return;
  lockLoading.value = true;
  error.value = "";
  try {
    const updated = await setViewClearSecs(secs);
    appConfig.value = updated;
    applySecurityConfig(updated);
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("settings.clear.setViewFailed");
  } finally {
    lockLoading.value = false;
  }
}

async function onClipboardClearChange(secs: number | null) {
  if (!appConfig.value) return;
  lockLoading.value = true;
  error.value = "";
  try {
    appConfig.value = await setClipboardClearSecs(secs);
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("settings.clear.setClipboardFailed");
  } finally {
    lockLoading.value = false;
  }
}

function openPassphraseModal(mode: PassphraseMode) {
  ppCurrent.value = "";
  ppNew.value = "";
  ppAck.value = false;
  error.value = "";
  passphraseModal.value = mode;
}

function closePassphraseModal() {
  ppCurrent.value = "";
  ppNew.value = "";
  ppAck.value = false;
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
    error.value = t("settings.passphrase.currentRequired");
    return;
  }
  if (ppShowCurrent.value && !ppCurrent.value) {
    error.value = t("settings.passphrase.required");
    return;
  }
  // set / change enter the new passphrase via PassphraseField (with a confirm
  // box); validate the two match before dispatching.
  if (ppShowNew.value) {
    // Defensive re-check: the submit button is already :disabled while unacked,
    // but this guards a future refactor that wraps the modal in a <form> (where
    // Enter could submit past a disabled button).
    if (!!ppNew.value && !ppAck.value) {
      error.value = t("settings.passphrase.ackRequired");
      return;
    }
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
      toast.success(t("settings.passphrase.setToast"));
    } else if (mode === "change") {
      await changePassphrase(ppCurrent.value, ppNew.value);
      biometricEnabled.value = await isBiometricUnlockEnabled();
      toast.success(t("settings.passphrase.changedToast"));
    } else if (mode === "enable-biometric") {
      await enableBiometricUnlock(ppCurrent.value, identityEnrollPrompt());
      biometricEnabled.value = true;
      toast.success(t("settings.biometric.enabledToast"));
    } else {
      await enableIdentityAutoUnlock(ppCurrent.value);
      identityAutoUnlockEnabled.value = true;
      toast.success(t("settings.appLock.autoUnlock.enabledToast"));
    }
    closePassphraseModal();
  } catch (e) {
    if (mode === "enable-biometric") {
      const err = e as BiometricError;
      if (err.code === "BIOMETRIC_CANCELLED") {
        // User cancelled the biometric prompt — keep the modal open for retry.
      } else if (err.code === "WRONG_PASSPHRASE") {
        error.value = t("settings.passphrase.wrongPassphrase");
      } else {
        error.value = err.message || t("settings.passphrase.biometricFailed");
      }
    } else if (mode === "enable-auto-unlock") {
      const err = asAppLockError(e) as AppLockError;
      error.value =
        err.code === "WRONG_PASSPHRASE"
          ? t("settings.passphrase.wrongPassphrase")
          : err.message || t("settings.passphrase.autoUnlockFailed");
    } else {
      const appError = e as AppError;
      error.value =
        appError?.message ||
        (mode === "set"
          ? t("settings.passphrase.setFailed")
          : t("settings.passphrase.changeFailed"));
    }
  } finally {
    passphraseLoading.value = false;
  }
}

async function onDisableBiometric() {
  await disableBiometricUnlock();
  biometricEnabled.value = false;
  toast.success(t("settings.biometric.disabledToast"));
}

// ── App-launch biometric gate (RFC 0028) ─────────────────────────────────
async function onEnableAppLock() {
  error.value = "";
  appLockLoading.value = true;
  try {
    await enableBiometricAppLock(appLockEnrollPrompt());
    appLockEnabled.value = true;
    toast.success(t("settings.appLock.enabledToast"));
  } catch (e) {
    const err = asAppLockError(e) as AppLockError;
    if (err.code === "BIOMETRIC_CANCELLED") {
      // User cancelled the migration prompt — no error toast.
    } else {
      error.value = err.message || t("settings.appLock.enableFailed");
    }
  } finally {
    appLockLoading.value = false;
  }
}

async function onDisableAppLock() {
  error.value = "";
  appLockLoading.value = true;
  try {
    await disableBiometricAppLock(appLockUnlockPrompt());
    appLockEnabled.value = false;
    // Disabling the gate makes identity auto-unlock moot.
    identityAutoUnlockEnabled.value = false;
    toast.success(t("settings.appLock.disabledToast"));
  } catch (e) {
    const err = asAppLockError(e) as AppLockError;
    if (err.code === "BIOMETRIC_CANCELLED") {
      // User cancelled — stays enabled.
    } else {
      error.value = err.message || t("settings.appLock.disableFailed");
    }
  } finally {
    appLockLoading.value = false;
  }
}

async function onDisableIdentityAutoUnlock() {
  await disableIdentityAutoUnlock();
  identityAutoUnlockEnabled.value = false;
  toast.success(t("settings.appLock.autoUnlock.disabledToast"));
}

// ── Repository authenticity ──────────────────────────────────────────────
async function loadAuthConfig() {
  try {
    // Warnings are a Settings-only concern (separate command, NOT on the
    // entry-list badge path); fetch alongside the config. A warnings failure
    // must not brick the whole card.
    const [cfg, warnings] = await Promise.all([
      getAuthenticityConfig(),
      getGpgKeyParseWarnings().catch(() => [] as string[]),
    ]);
    authConfig.value = cfg;
    gpgWarnings.value = warnings;
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("settings.auth.loadConfigFailed");
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
      error.value = t("settings.auth.enforceNeedsKey");
      // Revert the radio to the current effective mode.
      authConfig.value.mode = authConfig.value.mode;
    } else {
      error.value = appError?.message || t("settings.auth.setModeFailed");
    }
  } finally {
    authLoading.value = false;
  }
}

async function onRemoveKey(fingerprint: string, kind: "ssh" | "gpg") {
  if (!confirm(t("settings.auth.removeConfirm"))) return;
  authLoading.value = true;
  try {
    if (kind === "gpg") {
      await removeTrustedGpgKey(fingerprint);
    } else {
      await removeTrustedKey(fingerprint);
    }
    toast.success(t("settings.auth.removedToast"));
    await loadAuthConfig();
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("settings.auth.removeFailed");
  } finally {
    authLoading.value = false;
  }
}

async function onImportGpgKey() {
  // Prompt for a label up front; the backend pre-fills from the filename when
  // this is blank, so empty is fine too.
  const label = window.prompt(t("settings.auth.importGpgPrompt"), "");
  if (label === null) return;
  authLoading.value = true;
  error.value = "";
  try {
    await importTrustedGpgKeyFile(label.trim());
    toast.success(t("settings.auth.importGpgToast"));
    await loadAuthConfig();
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("settings.auth.importGpgFailed");
  } finally {
    authLoading.value = false;
  }
}

async function onTrustHead() {
  const label = window.prompt(t("settings.auth.trustHeadPrompt"), "signer");
  if (label === null) return;
  authLoading.value = true;
  try {
    await trustHeadSigner(label.trim() || "signer");
    toast.success(t("settings.auth.trustHeadToast"));
    await loadAuthConfig();
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("settings.auth.trustHeadFailed");
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
  // Gate the leave guard before the wipe — see onBeforeRouteLeave. Covers
  // navigation during the wipe: the reset modal's back is now handled by
  // BaseModalShell, but a nav tap — or back during the listener's
  // async-registration window — could still trip the guard.
  isResetting = true;
  try {
    await apiResetConfig();
    resetOpen.value = false;
    const failure = await router.replace({ name: "setup" });
    // vue-router resolves a cancelled/aborted nav as a NavigationFailure, not a
    // throw. The backend is already wiped — force a re-init so the app re-enters at
    // /setup instead of stranding the user on stale Settings.
    if (failure) window.location.reload();
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("settings.reset.failed");
    resetOpen.value = false;
  } finally {
    isResetting = false;
  }
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
const hasUnsavedChanges = computed(() => commitDirty.value);

// Commit the dirty commit-identity form. Returns false on failure so the leave
// guard can keep the user on the page to see the error (the handler sets `error`
// and leaves the form dirty on failure). The add-key form now lives on its own
// route, so it no longer contributes to this page's dirty state.
async function commitAllPending(): Promise<boolean> {
  if (!commitDirty.value) return true;
  return onSaveCommitIdentity();
}

type UnsavedChoice = "discard" | "save" | "cancel";
const unsavedOpen = ref(false);
let pendingResolve: ((c: UnsavedChoice) => void) | null = null;
// Re-entrancy guard: if a second navigation supersedes the guarded one while
// the modal is open, only the latest token's resolver runs; older ones no-op.
let promptToken = 0;
// Gate the unsaved-changes leave guard for the entire reset flow. Set BEFORE the
// apiResetConfig await: during the wipe, any navigation — a nav tap, or hardware
// Back during the reset modal's brief async-listener-registration window — could
// trip the guard before a post-wipe flag would exist. Cleared in doReset's finally.
let isResetting = false;

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
  // Reset in flight: unsaved edits are scoped to the store the wipe just destroyed,
  // so the Save/Keep/Discard prompt is incoherent. The gate covers the whole reset
  // (see doReset), not just the navigation.
  if (isResetting) return true;
  if (!hasUnsavedChanges.value) return true;
  const choice = await promptUnsaved();
  if (choice === "cancel") return false;
  if (choice === "save") {
    const ok = await commitAllPending();
    if (!ok) return false; // keep the user on the page so the error is visible
  }
  return true; // discard, or save succeeded
});

onMounted(() => {
  loadConfig();
  loadAuthConfig();
  loadLocalePref();
});
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <BaseHeader
      :back-fallback="{ name: 'entries' }"
      :title="t('settings.title')"
      :title-icon="Settings"
    />

    <BaseCard as="section" class="mb-4">
      <h2 class="text-sm font-medium mb-2">
        {{ t("settings.language.title") }}
      </h2>
      <p class="text-xs text-muted mb-3">
        {{ t("settings.language.description") }}
      </p>
      <BaseSegmentedControl
        name="display-language"
        :legend="t('settings.language.legend')"
        :model-value="localeSelection"
        :options="[
          { label: t('settings.language.system'), value: 'system' },
          { label: t('settings.language.english'), value: 'en' },
          { label: t('settings.language.chinese'), value: 'zh-CN' },
        ]"
        @change="onLocaleChange"
      />
    </BaseCard>

    <div v-if="loading" class="text-center text-muted py-8">
      {{ t("common.loading") }}
    </div>

    <BaseAlert v-else-if="error" variant="danger" class="mb-4">
      {{ error }}
    </BaseAlert>

    <div v-else-if="config" class="flex flex-col gap-4">
      <!-- Repo info -->
      <BaseCard as="section">
        <h2 class="text-sm font-medium mb-2">{{ t("settings.repo.title") }}</h2>
        <div class="text-sm text-muted break-all">{{ config.url }}</div>
        <div class="text-xs text-muted mt-1">
          {{
            isSsh
              ? t("settings.repo.auth.ssh")
              : config.pat
                ? t("settings.repo.auth.pat")
                : t("settings.repo.auth.none")
          }}
        </div>
      </BaseCard>

      <!-- Commit identity -->
      <BaseCard as="section" :border="commitDirty ? 'accent' : 'edge'">
        <h2 class="text-sm font-medium mb-2">
          {{ t("settings.commit.title") }}
          <span
            v-if="commitDirty"
            class="ml-1 text-xs font-normal"
            style="color: var(--color-accent)"
            >{{ t("settings.commit.unsaved") }}</span
          >
        </h2>
        <p class="text-xs text-muted mb-3">
          {{
            t("settings.commit.hint", {
              default: commitDefault
                ? t("settings.commit.default", {
                    name: commitDefault.name,
                    email: commitDefault.email,
                  })
                : "",
            })
          }}
        </p>
        <div class="flex flex-col gap-1 mb-3">
          <label for="commit-name" class="text-xs text-muted">{{
            t("settings.commit.nameLabel")
          }}</label>
          <BaseInput
            id="commit-name"
            v-model="commitName"
            type="text"
            :placeholder="t('settings.commit.namePlaceholder')"
            autocomplete="off"
            :disabled="commitLoading"
          />
        </div>
        <div class="flex flex-col gap-1 mb-3">
          <label for="commit-email" class="text-xs text-muted">{{
            t("settings.commit.emailLabel")
          }}</label>
          <BaseInput
            id="commit-email"
            v-model="commitEmail"
            type="email"
            :placeholder="t('settings.commit.emailPlaceholder')"
            autocomplete="off"
            :disabled="commitLoading"
          />
        </div>
        <BaseButton
          variant="action"
          :loading="commitLoading"
          @click="onSaveCommitIdentity"
        >
          {{ t("settings.commit.save") }}
        </BaseButton>
      </BaseCard>

      <!-- SSH key management — the key view/export lives on its own route so
           Android back returns here instead of the entry list. -->
      <BaseCard as="section" v-if="isSsh">
        <h2 class="text-sm font-medium mb-3">{{ t("settings.ssh.title") }}</h2>
        <BaseButton variant="action" @click="router.push({ name: 'sshKey' })">
          <BaseIcon :icon="KeyRound" /> {{ t("settings.ssh.manage") }}
        </BaseButton>
      </BaseCard>

      <!-- Passphrase management (x25519 identities only — SSH keys rely on
           their own native passphrase protection). Set / change run in the
           shared passphrase modal, which is the commit boundary. -->
      <BaseCard as="section" v-if="!isSshIdentity">
        <h2 class="text-sm font-medium mb-3">
          {{ t("settings.passphrase.title") }}
        </h2>

        <!-- Not encrypted: set passphrase -->
        <template v-if="!isIdentityEncrypted">
          <p class="text-xs text-muted mb-2">
            {{ t("settings.passphrase.plaintextHint") }}
          </p>
          <BaseButton variant="action" @click="openPassphraseModal('set')">
            <BaseIcon :icon="Lock" /> {{ t("settings.passphrase.set") }}
          </BaseButton>
        </template>

        <!-- Encrypted: change passphrase -->
        <template v-else>
          <p class="text-xs text-muted mb-2 flex items-center gap-1">
            <BaseIcon :icon="CircleCheck" :size="14" class="text-success" />
            {{ t("settings.passphrase.encryptedHint") }}
          </p>
          <BaseButton variant="action" @click="openPassphraseModal('change')">
            <BaseIcon :icon="KeyRound" /> {{ t("settings.passphrase.change") }}
          </BaseButton>
        </template>
      </BaseCard>

      <!-- SSH key identities are not encrypted by gpm -->
      <BaseCard as="section" v-else>
        <h2 class="text-sm font-medium mb-3">
          {{ t("settings.passphrase.titleEncrypted") }}
        </h2>
        <p class="text-xs text-muted">
          {{ t("settings.passphrase.sshIdentityHint") }}
        </p>
      </BaseCard>

      <!-- Biometric unlock (only meaningful when the identity is encrypted) -->
      <BaseCard as="section" v-if="isIdentityEncrypted">
        <h2 class="text-sm font-medium mb-3">
          {{ t("settings.biometric.title") }}
        </h2>

        <p v-if="!biometricAvailable" class="text-xs text-muted">
          {{ t("settings.biometric.unavailable") }}
        </p>

        <template v-else-if="!biometricEnabled">
          <p class="text-xs text-muted mb-2">
            {{ t("settings.biometric.enableHint") }}
          </p>
          <BaseButton
            variant="action"
            :disabled="biometricLoading"
            @click="openPassphraseModal('enable-biometric')"
          >
            {{ t("settings.biometric.enable") }}
          </BaseButton>
        </template>

        <template v-else>
          <p class="text-xs text-muted mb-2">
            {{ t("settings.biometric.enabledHint") }}
          </p>
          <BaseButton variant="action-danger" @click="onDisableBiometric">
            {{ t("settings.biometric.disable") }}
          </BaseButton>
        </template>
      </BaseCard>

      <!-- App-launch biometric gate (RFC 0028) -->
      <BaseCard as="section" v-if="appLockAvailable">
        <h2 class="text-sm font-medium mb-3">
          {{ t("settings.appLock.title") }}
        </h2>
        <p class="text-xs text-muted mb-3">
          {{ t("settings.appLock.description") }}
        </p>

        <!-- App lock enable/disable -->
        <template v-if="!appLockEnabled">
          <BaseButton
            variant="action"
            :loading="appLockLoading"
            @click="onEnableAppLock"
          >
            <BaseIcon :icon="Lock" /> {{ t("settings.appLock.enable") }}
          </BaseButton>
        </template>

        <template v-else>
          <p class="text-xs text-muted mb-2 flex items-center gap-1">
            <BaseIcon :icon="CircleCheck" :size="14" class="text-success" />
            {{ t("settings.appLock.enabledHint") }}
          </p>
          <BaseButton
            variant="action-danger"
            :disabled="appLockLoading"
            @click="onDisableAppLock"
          >
            {{ t("settings.appLock.disable") }}
          </BaseButton>

          <!-- Identity auto-unlock opt-in (req3): separate from the auto-lock
               timing presets below; only meaningful with the gate on and an
               encrypted identity. -->
          <div
            v-if="isIdentityEncrypted"
            class="mt-4 pt-4 border-t border-edge"
          >
            <h3 class="text-sm font-medium mb-1">
              {{ t("settings.appLock.autoUnlock.title") }}
            </h3>
            <p class="text-xs text-muted mb-3">
              {{ t("settings.appLock.autoUnlock.description") }}
            </p>
            <template v-if="!identityAutoUnlockEnabled">
              <BaseButton
                variant="action"
                :disabled="appLockLoading"
                @click="openPassphraseModal('enable-auto-unlock')"
              >
                {{ t("settings.appLock.autoUnlock.enable") }}
              </BaseButton>
            </template>
            <template v-else>
              <p class="text-xs text-muted mb-2">
                {{ t("settings.appLock.autoUnlock.enabledHint") }}
              </p>
              <BaseButton
                variant="action-danger"
                :disabled="appLockLoading"
                @click="onDisableIdentityAutoUnlock"
              >
                {{ t("settings.appLock.autoUnlock.disable") }}
              </BaseButton>
            </template>
          </div>
        </template>
      </BaseCard>

      <!-- Screen capture protection (Android FLAG_SECURE) — Android only -->
      <BaseCard as="section" v-if="secureAvailable">
        <h2 class="text-sm font-medium mb-2">
          {{ t("settings.secureScreen.title") }}
        </h2>
        <p class="text-xs text-muted mb-3">
          {{ t("settings.secureScreen.description") }}
        </p>
        <BaseSegmentedControl
          name="secure-screen"
          :legend="t('settings.secureScreen.legend')"
          :model-value="secureScreen"
          :options="[
            { label: t('settings.secureScreen.on'), value: true },
            { label: t('settings.secureScreen.off'), value: false },
          ]"
          @change="onSecureScreenChange"
        >
          <template #hint>
            <p class="text-xs text-muted mt-1">
              <template v-if="secureScreen">{{
                t("settings.secureScreen.onHint")
              }}</template>
              <template v-else>{{
                t("settings.secureScreen.offHint")
              }}</template>
            </p>
          </template>
        </BaseSegmentedControl>
      </BaseCard>

      <!-- AutoSync -->
      <BaseCard as="section" v-if="config">
        <h2 class="text-sm font-medium mb-3">
          {{ t("settings.autosync.title") }}
        </h2>
        <BaseSegmentedControl
          class="mb-3"
          name="autosync"
          :legend="t('settings.autosync.legend')"
          :model-value="autosyncEnabled"
          :options="[
            { label: t('settings.autosync.on'), value: true },
            { label: t('settings.autosync.off'), value: false },
          ]"
          :disabled="lockLoading"
          @change="onAutosyncChange"
        >
          <template #hint>
            <p class="text-xs text-muted mt-1">
              <template v-if="autosyncEnabled">{{
                t("settings.autosync.onHint")
              }}</template>
              <template v-else>{{ t("settings.autosync.offHint") }}</template>
            </p>
          </template>
        </BaseSegmentedControl>
      </BaseCard>

      <!-- Auto-lock & auto-clear -->
      <BaseCard as="section" v-if="config">
        <h2 class="text-sm font-medium mb-3">
          {{ t("settings.lock.title") }}
        </h2>
        <p class="text-xs text-muted mb-3">
          {{ t("settings.lock.description") }}
        </p>

        <!-- App auto-lock mode -->
        <BaseSegmentedControl
          class="mb-3"
          name="lock-mode"
          :legend="t('settings.lock.autoLockLegend')"
          wrap
          :model-value="rawLockMode"
          :by="lockModeEq"
          :options="LOCK_PRESETS"
          :disabled="lockLoading"
          @change="onLockModeChange"
        >
          <template #hint>
            <p class="text-xs text-muted mt-1">
              <template v-if="lockModeActive('immediate')">{{
                t("settings.lock.immediateHint")
              }}</template>
              <template v-else-if="lockModeActive('never')">{{
                t("settings.lock.neverHint")
              }}</template>
              <template v-else>{{ t("settings.lock.idleHint") }}</template>
            </p>
          </template>
        </BaseSegmentedControl>

        <!-- View auto-clear -->
        <BaseSegmentedControl
          class="mb-3"
          name="view-clear"
          :legend="t('settings.lock.viewClearLegend')"
          wrap
          :model-value="rawViewClear"
          :options="VIEW_CLEAR_PRESETS"
          :disabled="lockLoading"
          @change="onViewClearChange"
        />

        <!-- Clipboard auto-clear -->
        <BaseSegmentedControl
          name="clipboard-clear"
          :legend="t('settings.lock.clipboardClearLegend')"
          wrap
          :model-value="rawClipboardClear"
          :options="CLIPBOARD_CLEAR_PRESETS"
          :disabled="lockLoading"
          @change="onClipboardClearChange"
        />
      </BaseCard>

      <!-- Repository authenticity -->
      <BaseCard as="section" v-if="authConfig">
        <h2 class="text-sm font-medium mb-3">
          {{ t("settings.auth.title") }}
        </h2>
        <p class="text-xs text-muted mb-3">
          {{ t("settings.auth.description") }}
        </p>

        <!-- Mode selector -->
        <BaseSegmentedControl
          class="mb-3"
          name="verify-mode"
          :legend="t('settings.auth.legend')"
          :model-value="authConfig.mode"
          :options="VERIFY_MODES"
          :disabled="authLoading"
          @change="onModeChange"
        >
          <template #hint>
            <p class="text-xs text-muted mt-1">
              <template v-if="authConfig.mode === 'off'">{{
                t("settings.auth.offHint")
              }}</template>
              <template v-else-if="authConfig.mode === 'audit'">{{
                t("settings.auth.auditHint")
              }}</template>
              <template v-else>{{ t("settings.auth.enforceHint") }}</template>
            </p>
          </template>
        </BaseSegmentedControl>

        <!-- Trusted signing keys (SSH + GPG combined; GPG entries tagged) -->
        <div class="text-xs text-muted mb-1">
          {{ t("settings.auth.trustedKeys", { count: trustedKeyRows.length }) }}
        </div>
        <ul v-if="trustedKeyRows.length" class="flex flex-col gap-1 mb-2">
          <li
            v-for="row in trustedKeyRows"
            :key="row.kind + ':' + row.fingerprint"
            class="key-row"
          >
            <code class="text-xs break-all flex-1">{{ row.fingerprint }}</code>
            <span
              v-if="row.kind === 'gpg'"
              class="text-[0.6rem] text-default px-1 rounded-sm bg-edge shrink-0"
              >GPG</span
            >
            <span class="text-xs text-muted mx-2 truncate">{{
              row.label
            }}</span>
            <button
              type="button"
              class="btn-copy"
              @click="onRemoveKey(row.fingerprint, row.kind)"
            >
              {{ t("settings.auth.remove") }}
            </button>
          </li>
        </ul>
        <p v-else class="text-xs text-muted mb-2">
          {{ t("settings.auth.noTrustedKeys") }}
        </p>
        <p
          v-if="gpgWarnings.length"
          class="text-xs text-warning mb-2 break-words"
        >
          {{ t("settings.auth.gpgWarning", { count: gpgWarnings.length }) }}
        </p>

        <div class="flex flex-col gap-2">
          <BaseButton
            v-if="trustedKeyRows.length === 0"
            variant="action"
            @click="onTrustHead"
          >
            <BaseIcon :icon="KeyRound" /> {{ t("settings.auth.trustHead") }}
          </BaseButton>
          <BaseButton variant="action" @click="router.push({ name: 'addKey' })">
            <BaseIcon :icon="Plus" /> {{ t("settings.auth.addKey") }}
          </BaseButton>
          <BaseButton
            variant="action"
            :disabled="authLoading"
            @click="onImportGpgKey"
          >
            <BaseIcon :icon="FileUp" /> {{ t("settings.auth.importGpg") }}
          </BaseButton>
          <BaseButton variant="action" @click="openHistory">
            <BaseIcon :icon="History" /> {{ t("settings.auth.viewHistory") }}
          </BaseButton>
        </div>
      </BaseCard>

      <!-- Danger zone -->
      <BaseCard as="section" border="danger">
        <h2 class="text-sm font-medium mb-2 text-danger">
          {{ t("settings.reset.title") }}
        </h2>
        <BaseButton variant="action-danger" @click="resetConfig">
          <BaseIcon :icon="Trash2" /> {{ t("settings.reset.button") }}
        </BaseButton>
        <p class="text-xs text-muted mt-1">
          {{ t("settings.reset.description") }}
        </p>
      </BaseCard>
    </div>

    <!-- Reset confirmation: type RESET to confirm (z=80 stacks above UnlockModal). -->
    <BaseModalShell
      v-if="resetOpen"
      variant="center"
      :z="80"
      role="alertdialog"
      :aria-label="t('settings.reset.ariaLabel')"
      @close="resetOpen = false"
    >
      <h2 class="text-lg font-medium text-danger mb-3">
        {{ t("settings.reset.modalTitle") }}
      </h2>
      <BaseAlert variant="danger" class="mb-4">
        {{ t("settings.reset.modalBody") }}
      </BaseAlert>
      <div class="flex flex-col gap-1 mb-4">
        <label class="text-sm font-medium" for="reset-confirm">{{
          t("settings.reset.typeReset")
        }}</label>
        <BaseInput
          id="reset-confirm"
          v-model="resetConfirmText"
          autocomplete="off"
          autofocus
        />
      </div>
      <div class="flex gap-2 justify-end">
        <BaseButton variant="secondary" @click="resetOpen = false">{{
          t("common.button.cancel")
        }}</BaseButton>
        <BaseButton variant="danger" :disabled="!resetReady" @click="doReset">
          <BaseIcon :icon="Trash2" /> {{ t("settings.reset.confirm") }}
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
        <label for="pp-current" class="text-xs text-muted">{{
          t("settings.passphrase.currentLabel")
        }}</label>
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
        :label="
          passphraseModal === 'change'
            ? t('settings.passphrase.newLabel')
            : t('settings.passphrase.plainLabel')
        "
        :placeholder="t('settings.passphrase.newPlaceholder')"
        :optional="false"
        :disabled="passphraseLoading"
        class="mb-3"
      />
      <PassphraseUnrecoverableAck
        v-if="ppShowNew"
        v-model="ppAck"
        class="mb-3"
      />
      <div class="flex gap-2 justify-end">
        <BaseButton
          variant="secondary"
          :disabled="passphraseLoading"
          @click="closePassphraseModal"
          >{{ t("common.button.cancel") }}</BaseButton
        >
        <BaseButton
          variant="action"
          :loading="passphraseLoading"
          :disabled="ppSubmitDisabled"
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
      :aria-label="t('settings.unsaved.ariaLabel')"
      @close="resolveUnsaved('cancel')"
    >
      <h2 class="text-lg font-medium mb-2">
        {{ t("settings.unsaved.title") }}
      </h2>
      <p class="text-sm text-muted mb-4">{{ t("settings.unsaved.body") }}</p>
      <div class="flex flex-col gap-2">
        <BaseButton variant="action" @click="resolveUnsaved('save')">{{
          t("settings.unsaved.save")
        }}</BaseButton>
        <BaseButton variant="secondary" @click="resolveUnsaved('cancel')">{{
          t("settings.unsaved.keep")
        }}</BaseButton>
        <BaseButton
          variant="action-danger"
          @click="resolveUnsaved('discard')"
          >{{ t("settings.unsaved.discard") }}</BaseButton
        >
      </div>
    </BaseModalShell>
  </main>
</template>

<style scoped>
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
