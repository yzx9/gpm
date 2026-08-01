<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import type {
  AppConfig,
  AppError,
  AppLockError,
  BiometricError,
  GateIdle,
  LockMode,
} from "@/api";
import {
  DEFAULT_GATE_IDLE,
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
  getConfig,
  isAppLockAvailable,
  isBiometricAvailable,
  isBiometricUnlockEnabled,
  setClipboardClearSecs,
  setGateIdle,
  setLockMode,
  setPassphrase,
  setViewClearSecs,
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
  useSecureClaim,
  useSecuritySettings,
  useToast,
  useWipeOnLeave,
} from "@/composables";
import {
  appLockEnrollPrompt,
  appLockUnlockPrompt,
  identityEnrollPrompt,
} from "@/i18n/native";
import { CircleCheck, KeyRound, Lock } from "@lucide/vue";
import { computed, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

const { toast } = useToast();
const { t } = useI18n();
const { applySecurityConfig } = useSecuritySettings();

const loading = ref(false);
const error = ref("");

// Behavior prefs (auto-lock mode + view/clipboard auto-clear) for the merged
// Auto-Lock card; each control writes back here + syncs the shared
// security-settings cache the activity bumper reads.
const appConfig = ref<AppConfig | null>(null);
// App-launch-gate in-app idle timeout ("Re-lock when inactive"). A computed view
// over appConfig (mirrors rawLockMode etc.) so it stays in sync automatically;
// the shared security-settings cache also holds it for the activity bumper.
const gateIdle = computed<GateIdle>(
  () => appConfig.value?.gate_idle ?? DEFAULT_GATE_IDLE,
);

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
// R031: the passphrase modal collects a credential, so hold a screen-capture
// claim only while it's open (the rest of this page carries no secret).
const { acquire: acquireSecure, release: releaseSecure } = useSecureClaim();
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

// Whether the stored identity is an SSH key. SSH keys are never
// passphrase-encrypted by gpm (they rely on their own native protection),
// so the seal encryption UI is hidden for them.
const isSshIdentity = computed(
  () =>
    identityType.value === "ssh_ed25519" || identityType.value === "ssh_rsa",
);

// Identity Auto-Unlock is on with the gate enabled and an encryptable identity:
// the identity then follows the gate's lifecycle, so its own auto-lock timing is
// ignored (the backend disarms reset_lock_timer + skips the Immediate wipe).
const identityCoupled = computed(
  () =>
    appLockEnabled.value &&
    identityAutoUnlockEnabled.value &&
    isIdentityEncrypted.value,
);

/** Wipe every in-DOM secret: the typed passphrase-modal inputs and their
 *  confirm echo. Idempotent — fires on a hard lock, browser back, and unmount.
 *  Also releases the modal's screen-capture claim (closePassphraseModal is the
 *  other dismiss path; this covers a hard lock while the modal is open). */
function wipeSecrets() {
  ppCurrent.value = "";
  ppNew.value = "";
  ppAck.value = false;
  passphraseModal.value = null;
  ppField.value?.reset();
  releaseSecure();
}

// The unlock modal keeps this page mounted on auto-lock, so unmount alone can't
// guarantee a wipe — clear on a hard lock, on browser back, and on unmount.
useWipeOnLeave(wipeSecrets);

async function loadConfig() {
  loading.value = true;
  error.value = "";
  try {
    const auth = await getAuthState();
    isIdentityEncrypted.value = auth.encrypted;
    identityType.value = auth.identity_type;
    biometricAvailable.value = (await isBiometricAvailable()) === "available";
    biometricEnabled.value = await isBiometricUnlockEnabled();
    appLockAvailable.value = await isAppLockAvailable();
    // The app-lock toggle reads Keystore truth (Path B), not the persisted
    // config flag — the two can drift, and the runtime gate is what matters.
    appLockEnabled.value = (await getAppLockState()).enabled;
    const cfg = await getConfig();
    identityAutoUnlockEnabled.value = cfg.unlock_identity_with_app ?? false;
    // Behavior prefs for the merged Auto-Lock card. Push them into the shared
    // security-settings cache too so the activity bumper reflects them.
    appConfig.value = await getAppConfig();
    applySecurityConfig(appConfig.value);
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("settings.passphrase.setFailed");
  } finally {
    loading.value = false;
  }
}

async function openPassphraseModal(mode: PassphraseMode) {
  ppCurrent.value = "";
  ppNew.value = "";
  ppAck.value = false;
  error.value = "";
  // Raise FLAG_SECURE BEFORE the credential inputs render, so an autofilled
  // current-passphrase is never on screen in the unprotected window. Fail-closed:
  // if the acquire IPC fails, don't open the modal.
  if (!(await acquireSecure())) {
    error.value = t("common.toast.secureScreenFailed");
    return;
  }
  passphraseModal.value = mode;
}

function closePassphraseModal() {
  ppCurrent.value = "";
  ppNew.value = "";
  ppAck.value = false;
  passphraseModal.value = null;
  releaseSecure();
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
      // Re-encryption can invalidate a previously-sealed biometric unlock and
      // auto-unlock slot — re-read both from the backend.
      biometricEnabled.value = await isBiometricUnlockEnabled();
      identityAutoUnlockEnabled.value =
        (await getConfig()).unlock_identity_with_app ?? false;
      toast.success(t("settings.passphrase.setToast"));
    } else if (mode === "change") {
      await changePassphrase(ppCurrent.value, ppNew.value);
      biometricEnabled.value = await isBiometricUnlockEnabled();
      identityAutoUnlockEnabled.value =
        (await getConfig()).unlock_identity_with_app ?? false;
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

// ── Auto-Lock & Auto-Clear ───────────────────────────────────────────────
// The identity's own auto-lock timing + the secret auto-clear windows. Each
// control writes the behavior pref and syncs the shared security-settings cache
// (clipboard-clear is the exception — it has no bumper dependency).
const lockLoading = ref(false);

const LOCK_PRESETS = computed<{ label: string; value: LockMode }[]>(() => [
  { label: t("settings.lock.immediate"), value: "immediate" },
  { label: t("settings.lock.minutes", { count: 1 }), value: { idle: 60 } },
  { label: t("settings.lock.minutes", { count: 5 }), value: { idle: 300 } },
  { label: t("settings.lock.minutes", { count: 15 }), value: { idle: 900 } },
  { label: t("settings.lock.minutes", { count: 30 }), value: { idle: 1800 } },
  { label: t("settings.lock.never"), value: "never" },
]);
const VIEW_CLEAR_PRESETS = computed<{ label: string; value: number | null }[]>(
  () => [
    { label: t("settings.clear.seconds", { count: 10 }), value: 10 },
    { label: t("settings.clear.default", { count: 45 }), value: null },
    { label: t("settings.lock.minutes", { count: 3 }), value: 180 },
    { label: t("settings.lock.never"), value: 0 },
  ],
);
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
// to BaseSegmentedControl's `by` prop. `lockModeActive` wraps it for hint checks.
function lockModeEq(a: LockMode, b: LockMode): boolean {
  if (a === b) return true;
  if (typeof a === "object" && typeof b === "object") return a.idle === b.idle;
  return false;
}
function lockModeActive(p: LockMode): boolean {
  return lockModeEq(rawLockMode.value, p);
}

async function onLockModeChange(mode: LockMode) {
  if (!appConfig.value) return;
  lockLoading.value = true;
  error.value = "";
  try {
    appConfig.value = await setLockMode(mode);
    applySecurityConfig(appConfig.value);
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("settings.lock.setModeFailed");
  } finally {
    lockLoading.value = false;
  }
}

async function onViewClearChange(secs: number | null) {
  if (!appConfig.value) return;
  lockLoading.value = true;
  error.value = "";
  try {
    appConfig.value = await setViewClearSecs(secs);
    applySecurityConfig(appConfig.value);
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

// ── Re-lock when inactive (app-launch-gate idle timer, R057) ─────────────
// Only meaningful with the gate on (the control lives in the App Lock card).
// The gate wipe is a superset of the identity auto-lock (it clears the master
// key too), so when both run the gate dominates — surfaced via copy, not by
// suppressing either control.
const GATE_IDLE_PRESETS = computed<{ label: string; value: GateIdle }[]>(() => [
  { label: t("settings.appLock.gateIdle.off"), value: "off" },
  { label: t("settings.lock.minutes", { count: 5 }), value: { after: 300 } },
  { label: t("settings.lock.minutes", { count: 15 }), value: { after: 900 } },
  { label: t("settings.lock.minutes", { count: 30 }), value: { after: 1800 } },
]);

// Object-valued (the { after } presets) → === never matches; use a comparator.
function gateIdleEq(a: GateIdle, b: GateIdle): boolean {
  if (a === b) return true;
  if (typeof a === "object" && typeof b === "object")
    return a.after === b.after;
  return false;
}

async function onGateIdleChange(mode: GateIdle) {
  if (!appConfig.value) return;
  appLockLoading.value = true;
  error.value = "";
  try {
    appConfig.value = await setGateIdle(mode);
    applySecurityConfig(appConfig.value);
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("settings.appLock.gateIdle.setFailed");
  } finally {
    appLockLoading.value = false;
  }
}

onMounted(() => {
  loadConfig();
});
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <BaseHeader
      :back-fallback="{ name: 'settings' }"
      :title="t('settings.hub.lockAndIdentity')"
      :title-icon="Lock"
    />

    <div v-if="loading" class="text-center text-muted py-8">
      {{ t("common.loading") }}
    </div>

    <BaseAlert v-else-if="error" variant="danger" class="mb-4">
      {{ error }}
    </BaseAlert>

    <div v-else class="flex flex-col gap-4">
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

          <!-- Re-lock when inactive (R057): the gate's own idle timer. Only
               meaningful with the gate on, so it lives here. The gate wipe also
               clears any unlocked passwords and dominates the identity auto-lock
               when both run. -->
          <div class="mt-4 pt-4 border-t border-edge">
            <h3 class="text-sm font-medium mb-1">
              {{ t("settings.appLock.gateIdle.legend") }}
            </h3>
            <p class="text-xs text-muted mb-3">
              {{ t("settings.appLock.gateIdle.description") }}
            </p>
            <BaseSegmentedControl
              name="gate-idle"
              wrap
              :model-value="gateIdle"
              :by="gateIdleEq"
              :options="GATE_IDLE_PRESETS"
              :disabled="appLockLoading"
              @change="onGateIdleChange"
            >
              <template #hint>
                <p class="text-xs text-muted mt-1">
                  <template v-if="gateIdle === 'off'">{{
                    t("settings.appLock.gateIdle.offHint")
                  }}</template>
                  <template v-else>{{
                    t("settings.appLock.gateIdle.idleHint")
                  }}</template>
                </p>
              </template>
            </BaseSegmentedControl>
          </div>

          <!-- Identity auto-unlock opt-in (req3): when on, the identity follows
               the gate's lifecycle and the Auto-Lock timing below is ignored.
               Only meaningful with the gate on and an encrypted identity. -->
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

      <!-- Auto-lock & auto-clear: the identity's own auto-lock timing + the
           identity's own auto-lock timing + the secret auto-clear windows. -->
      <BaseCard as="section">
        <h2 class="text-sm font-medium mb-3">
          {{ t("settings.lock.title") }}
        </h2>
        <p class="text-xs text-muted mb-3">
          {{ t("settings.lock.description") }}
        </p>

        <!-- Identity auto-lock mode (disabled while the identity is coupled to
             App Lock — the gate then owns its lifecycle; see the note below). -->
        <BaseSegmentedControl
          class="mb-3"
          name="lock-mode"
          :legend="t('settings.lock.autoLockLegend')"
          wrap
          :model-value="rawLockMode"
          :by="lockModeEq"
          :options="LOCK_PRESETS"
          :disabled="identityCoupled || lockLoading"
          :aria-describedby="identityCoupled ? 'lock-mode-managed' : undefined"
          @change="onLockModeChange"
        >
          <template #hint>
            <p
              v-if="identityCoupled"
              id="lock-mode-managed"
              class="text-xs text-muted mt-1"
            >
              {{ t("settings.lock.managedByAppLock") }}
            </p>
            <p v-else class="text-xs text-muted mt-1">
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

        <!-- Password view auto-clear -->
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
    </div>

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
  </main>
</template>
