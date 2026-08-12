<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<script setup lang="ts">
import type { BiometricError } from "@/api";
import {
  asBiometricError,
  biometricUnlock,
  disableBiometricUnlock,
  getAppConfig,
  isBiometricAvailable,
  isBiometricUnlockEnabled,
  unlock,
  type LockMode,
} from "@/api";
import { useActiveRepo, useWipeOnLeave, Z } from "@/composables";
import { reconcileLocaleFromBackend } from "@/i18n";
import { identityUnlockPrompt } from "@/i18n/native";
import { HelpCircle, LockKeyhole, ScanFace, X } from "@lucide/vue";
import { computed, nextTick, onMounted, onUnmounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import BaseAlert from "./base/BaseAlert.vue";
import BaseButton from "./base/BaseButton.vue";
import BaseIcon from "./base/BaseIcon.vue";
import BaseInput from "./base/BaseInput.vue";
import BaseModalShell from "./base/BaseModalShell.vue";

const props = withDefaults(
  defineProps<{
    /** Whether to auto-fire the system biometric prompt on mount. Suppressed
     *  for an idle-timeout re-lock (the user likely stepped away) — the overlay
     *  still renders in biometric mode so the user can tap to unlock. */
    autoPromptBiometric?: boolean;
  }>(),
  { autoPromptBiometric: true },
);

const emit = defineEmits<{ (e: "close"): void }>();

const { t } = useI18n();

const passphrase = ref("");
const loading = ref(false);
const error = ref("");
const showHelp = ref(false);

// Wipe the typed passphrase on browser back and on unmount — both exit paths
// (success unmounts via App.vue's lock-driven v-if; dismiss via @close) — so it
// isn't left for GC. No lock wiring: this IS the lock UI.
useWipeOnLeave(
  () => {
    passphrase.value = "";
  },
  { lock: false },
);

// ── Unlock method ─────────────────────────────────────────────────────
// Two modes: biometric (the default path when available) and passphrase
// (revealed on demand). `resolved` gates the interactive body until onMounted
// has chosen the mode, so the wrong branch never paints for a frame.
const mode = ref<"biometric" | "passphrase">("passphrase");
const resolved = ref(false);
const passphraseInputRef = ref<{ focus: () => void } | null>(null);

// ── Biometric state ───────────────────────────────────────────────────
const biometricAvailable = ref(false);
const biometricEnabled = ref(false);
const biometricLoading = ref(false);
const biometricNotice = ref("");
const biometricUsable = computed(
  () => biometricAvailable.value && biometricEnabled.value,
);

// ── Auto-lock policy hint ─────────────────────────────────────────────
// The policy in effect (Immediate / N min idle / Never), shown so the user
// knows how long the identity stays cached after unlocking. Defaults to
// "immediate" (the backend default) until getAppConfig() resolves; a fetch
// failure leaves that default in place.
const lockMode = ref<LockMode>("immediate");
const lockHint = computed(() => describeLockMode(lockMode.value));
function describeLockMode(m: LockMode): string {
  if (m === "immediate") return t("common.unlock.lockModeImmediate");
  if (m === "never") return t("common.unlock.lockModeNever");
  const mins = Math.round(m.idle / 60);
  return t("common.unlock.lockModeIdle", { mins });
}

// Single path into passphrase mode — used by both the switch tap and the
// error-driven auto-fallback. Clears stale status, flips mode, and focuses
// the revealed input (the native `autofocus` attribute does not fire when an
// input is mounted dynamically via v-if).
function enterPassphraseMode() {
  error.value = "";
  mode.value = "passphrase";
  nextTick(() => passphraseInputRef.value?.focus());
}

function switchToBiometric() {
  error.value = "";
  biometricNotice.value = "";
  mode.value = "biometric";
  // Re-prompt; the native biometric sheet handles its own focus.
  tryBiometricUnlock();
}

const activeRepo = useActiveRepo();

async function tryBiometricUnlock() {
  biometricNotice.value = "";
  biometricLoading.value = true;
  try {
    // Authoritative locale before building prompt text — same reason as
    // AppLockOverlay: a pinned-preference user's first prompt must use the
    // pinned locale, not the boot/system guess. This modal auto-prompts on
    // mount, so it can't rely on main.ts's fire-and-forget reconcile.
    await reconcileLocaleFromBackend();
    const repoId = await activeRepo.currentId();
    await biometricUnlock(repoId, identityUnlockPrompt());
    // Success: the backend emits `identity-lock-state { locked: false }`, which
    // App.vue's `v-if` reacts to and unmounts this overlay. Nothing to do here.
  } catch (e) {
    const err = asBiometricError(e) as BiometricError;
    switch (err.code) {
      case "KEYSTORE_CANCELLED":
        // User dismissed the prompt — stay in the current mode. The visible
        // ghost switch is the way to the other method; no notice needed.
        break;
      case "KEYSTORE_KEY_INVALIDATED":
        biometricNotice.value = t("common.unlock.biometricResetNotice");
        await disableBiometricUnlock();
        biometricEnabled.value = false;
        // Biometric is no longer viable — land on the passphrase form.
        enterPassphraseMode();
        break;
      case "WRONG_PASSPHRASE":
        biometricNotice.value = t("common.unlock.biometricStaleNotice");
        await disableBiometricUnlock();
        biometricEnabled.value = false;
        enterPassphraseMode();
        break;
      default:
        // Transient/unavailable (LOCKOUT, FAILED, …): keep biometric available
        // so the user can retry, or switch manually via the ghost button. The
        // backend's localized message surfaces as-is (the LOCKOUT test asserts
        // it), falling back to a generic notice.
        biometricNotice.value =
          err.message || t("common.unlock.biometricUnlockFailed");
    }
  } finally {
    biometricLoading.value = false;
  }
}

async function onUnlock() {
  error.value = "";

  if (!passphrase.value) {
    error.value = t("common.unlock.errRequired");
    return;
  }

  loading.value = true;
  try {
    const repoId = await activeRepo.currentId();
    await unlock(repoId, passphrase.value);
    // Success: the backend emits `identity-lock-state { locked: false }`, which
    // App.vue reacts to and unmounts this overlay. Nothing to do here.
  } catch (e) {
    const appError = e as { code?: string; message?: string };
    if (appError?.code === "WRONG_PASSPHRASE") {
      error.value = t("common.unlock.errWrong");
    } else {
      error.value = appError?.message || t("common.unlock.errUnlockFailed");
    }
  } finally {
    loading.value = false;
  }
}

// Reset is intentionally not offered from the unlock dialog: it is too dangerous
// for a surface users reach often. Recovery lives in Settings → Danger Zone
// (and, if the device's biometrics are all removed, via clearing app data /
// reinstalling — see AppLockOverlay for that dead-end guidance).

onMounted(async () => {
  console.info("[gpm:ui] unlock modal shown");
  biometricAvailable.value = (await isBiometricAvailable()) === "available";
  biometricEnabled.value = await isBiometricUnlockEnabled();
  // Pick the mode before un-gating so the first paint is correct (no flash of
  // the passphrase form on the biometric path), then auto-prompt if usable AND
  // the lock reason warrants it. An idle-timeout re-lock
  // (autoPromptBiometric == false) skips the system prompt — the user likely
  // stepped away, so it would just expire before they return — but stays in
  // biometric mode so the button is ready to tap.
  if (biometricUsable.value) mode.value = "biometric";
  resolved.value = true;
  if (biometricUsable.value && props.autoPromptBiometric) {
    await tryBiometricUnlock();
  } else if (!biometricUsable.value) {
    // Passphrase mode is the initial render here — focus the input ourselves
    // since `autofocus` doesn't fire on this dynamically (v-if) mounted field.
    nextTick(() => passphraseInputRef.value?.focus());
  }
  // Best-effort: read the auto-lock policy so the hint matches the user's
  // setting. A failure (or pre-setup) leaves the "immediate" default.
  try {
    lockMode.value = (await getAppConfig()).lock_mode ?? "immediate";
  } catch (e) {
    // keep default
    console.debug("[unlock-modal] lock-mode probe failed", e);
  }
});

onUnmounted(() => {
  console.info("[gpm:ui] unlock modal closed");
});
</script>

<template>
  <BaseModalShell
    variant="center"
    :z="Z.overlay"
    :aria-label="t('common.unlock.title')"
    @close="emit('close')"
  >
    <div class="title-row relative mb-1">
      <BaseButton
        variant="link"
        size="xs"
        tone="muted"
        class="absolute -top-1 -right-1"
        :aria-label="t('common.unlock.close')"
        @click="emit('close')"
      >
        <BaseIcon :icon="X" :size="18" />
      </BaseButton>
      <h1
        class="text-center text-display flex items-center justify-center gap-2"
      >
        <BaseIcon :icon="LockKeyhole" :size="28" /> gpm
        <BaseButton
          variant="link"
          size="xs"
          :tone="showHelp ? 'default' : 'muted'"
          :aria-expanded="showHelp"
          :aria-label="t('common.unlock.helpLabel')"
          @click="showHelp = !showHelp"
        >
          <BaseIcon :icon="HelpCircle" :size="16" />
        </BaseButton>
      </h1>
    </div>
    <p class="text-center text-muted text-sm mb-1">
      {{ t("common.unlock.locked") }}
    </p>
    <p class="text-center text-muted text-xs mb-6">{{ lockHint }}</p>

    <!-- What is the passphrase? (toggleable) -->
    <BaseAlert v-if="showHelp" variant="info" class="mb-4">
      {{ t("common.unlock.help") }}
    </BaseAlert>

    <!-- Biometric notice (reset / stale / failure) -->
    <BaseAlert
      v-if="biometricNotice"
      variant="danger"
      role="status"
      class="mb-4"
    >
      {{ biometricNotice }}
    </BaseAlert>

    <!-- BIOMETRIC MODE: primary biometric action + low-emphasis switch. -->
    <div v-if="resolved && mode === 'biometric'" class="flex flex-col gap-4">
      <BaseButton
        variant="primary"
        block
        :loading="biometricLoading"
        :disabled="loading"
        @click="tryBiometricUnlock"
      >
        <BaseIcon v-if="!biometricLoading" :icon="ScanFace" />
        <span>{{
          biometricLoading
            ? t("common.unlock.unlocking")
            : t("common.unlock.unlockWithBiometric")
        }}</span>
      </BaseButton>
      <BaseButton variant="ghost" block @click="enterPassphraseMode">
        {{ t("common.unlock.unlockWithPassphrase") }}
      </BaseButton>
    </div>

    <!-- PASSPHRASE MODE: input + primary + (optional) switch back to biometric. -->
    <form
      v-else-if="resolved"
      @submit.prevent="onUnlock"
      class="flex flex-col gap-4"
    >
      <div class="flex flex-col gap-1">
        <label for="passphrase" class="text-sm font-medium">{{
          t("common.unlock.passphraseLabel")
        }}</label>
        <BaseInput
          id="passphrase"
          ref="passphraseInputRef"
          v-model="passphrase"
          type="password"
          :placeholder="t('common.unlock.placeholder')"
          required
          autocomplete="off"
          :disabled="loading"
        />
        <small class="text-xs text-muted">{{
          t("common.unlock.passphraseHint")
        }}</small>
      </div>

      <BaseAlert v-if="error" variant="danger">{{ error }}</BaseAlert>

      <BaseButton variant="primary" type="submit" block :loading="loading">{{
        loading
          ? t("common.unlock.decrypting")
          : t("common.unlock.unlockButton")
      }}</BaseButton>

      <BaseButton
        v-if="biometricUsable"
        variant="ghost"
        block
        @click="switchToBiometric"
      >
        {{ t("common.unlock.unlockWithBiometric") }}
      </BaseButton>
    </form>
  </BaseModalShell>
</template>
