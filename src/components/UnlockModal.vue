<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import type { BiometricError } from "@/api";
import {
  asBiometricError,
  biometricUnlock,
  disableBiometricUnlock,
  getConfig,
  isBiometricAvailable,
  isBiometricUnlockEnabled,
  unlock,
  type LockMode,
} from "@/api";
import { HelpCircle, LockKeyhole, ScanFace, X } from "@lucide/vue";
import { computed, nextTick, onMounted, ref } from "vue";
import BaseAlert from "./base/BaseAlert.vue";
import BaseButton from "./base/BaseButton.vue";
import BaseIcon from "./base/BaseIcon.vue";
import BaseInput from "./base/BaseInput.vue";
import BaseModalShell from "./base/BaseModalShell.vue";

const emit = defineEmits<{ (e: "close"): void }>();

const passphrase = ref("");
const loading = ref(false);
const error = ref("");
const showHelp = ref(false);

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
// "immediate" (the backend default) until getConfig() resolves; a fetch
// failure leaves that default in place.
const lockMode = ref<LockMode>("immediate");
const lockHint = computed(() => describeLockMode(lockMode.value));
function describeLockMode(m: LockMode): string {
  if (m === "immediate") return "Identity is cleared after every action.";
  if (m === "never") return "Identity stays unlocked until you lock manually.";
  const mins = Math.round(m.idle / 60);
  return `Identity auto-locks after ${mins} min of inactivity.`;
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

async function tryBiometricUnlock() {
  biometricNotice.value = "";
  biometricLoading.value = true;
  try {
    await biometricUnlock();
    // Success: the backend emits `identity-lock-state { locked: false }`, which
    // App.vue's `v-if` reacts to and unmounts this overlay. Nothing to do here.
  } catch (e) {
    const err = asBiometricError(e) as BiometricError;
    switch (err.code) {
      case "BIOMETRIC_CANCELLED":
        // User dismissed the prompt — stay in the current mode. The visible
        // ghost switch is the way to the other method; no notice needed.
        break;
      case "BIOMETRIC_KEY_INVALIDATED":
        biometricNotice.value =
          "Biometric was reset (new fingerprint?) — re-enable it in Settings.";
        await disableBiometricUnlock();
        biometricEnabled.value = false;
        // Biometric is no longer viable — land on the passphrase form.
        enterPassphraseMode();
        break;
      case "WRONG_PASSPHRASE":
        biometricNotice.value =
          "Stored passphrase is invalid — re-enable biometric in Settings.";
        await disableBiometricUnlock();
        biometricEnabled.value = false;
        enterPassphraseMode();
        break;
      default:
        // Transient/unavailable (LOCKOUT, FAILED, …): keep biometric available
        // so the user can retry, or switch manually via the ghost button.
        biometricNotice.value = err.message || "Biometric unlock failed";
    }
  } finally {
    biometricLoading.value = false;
  }
}

async function onUnlock() {
  error.value = "";

  if (!passphrase.value) {
    error.value = "Passphrase is required";
    return;
  }

  loading.value = true;
  try {
    await unlock(passphrase.value);
    // Success: the backend emits `identity-lock-state { locked: false }`, which
    // App.vue reacts to and unmounts this overlay. Nothing to do here.
  } catch (e) {
    const appError = e as { code?: string; message?: string };
    if (appError?.code === "WRONG_PASSPHRASE") {
      error.value = "Wrong passphrase — please try again";
    } else {
      error.value = appError?.message || "Unlock failed";
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
  biometricAvailable.value = await isBiometricAvailable();
  biometricEnabled.value = await isBiometricUnlockEnabled();
  // Pick the mode before un-gating so the first paint is correct (no flash of
  // the passphrase form on the biometric path), then auto-prompt if usable.
  if (biometricUsable.value) mode.value = "biometric";
  resolved.value = true;
  if (biometricUsable.value) {
    await tryBiometricUnlock();
  } else {
    // Passphrase mode is the initial render here — focus the input ourselves
    // since `autofocus` doesn't fire on this dynamically (v-if) mounted field.
    nextTick(() => passphraseInputRef.value?.focus());
  }
  // Best-effort: read the auto-lock policy so the hint matches the user's
  // setting. A failure (or pre-setup) leaves the "immediate" default.
  try {
    lockMode.value = (await getConfig()).lock_mode ?? "immediate";
  } catch {
    // keep default
  }
});
</script>

<template>
  <BaseModalShell
    variant="center"
    :z="60"
    aria-label="Unlock identity"
    @close="emit('close')"
  >
    <div class="title-row relative mb-1">
      <button
        type="button"
        class="close-x"
        aria-label="Close"
        @click="emit('close')"
      >
        <BaseIcon :icon="X" :size="18" />
      </button>
      <h1
        class="text-center text-display flex items-center justify-center gap-2"
      >
        <BaseIcon :icon="LockKeyhole" :size="28" /> gpm
        <button
          type="button"
          class="help-btn"
          :class="{ active: showHelp }"
          :aria-expanded="showHelp"
          aria-label="What is this passphrase?"
          @click="showHelp = !showHelp"
        >
          <BaseIcon :icon="HelpCircle" :size="16" />
        </button>
      </h1>
    </div>
    <p class="text-center text-muted text-sm mb-1">Identity is locked</p>
    <p class="text-center text-muted text-xs mb-6">{{ lockHint }}</p>

    <!-- What is the passphrase? (toggleable) -->
    <BaseAlert v-if="showHelp" variant="info" class="mb-4">
      Your passphrase decrypts the identity that guards your secrets. It lives
      only on this device — gpm cannot recover or reset it for you. If you lose
      it, your secrets are permanently lost; you would need to reset gpm
      (Settings → Danger Zone) and set it up again.
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
          biometricLoading ? "Unlocking…" : "Unlock with biometric"
        }}</span>
      </BaseButton>
      <BaseButton variant="ghost" block @click="enterPassphraseMode">
        Unlock with passphrase
      </BaseButton>
    </div>

    <!-- PASSPHRASE MODE: input + primary + (optional) switch back to biometric. -->
    <form
      v-else-if="resolved"
      @submit.prevent="onUnlock"
      class="flex flex-col gap-4"
    >
      <div class="flex flex-col gap-1">
        <label for="passphrase" class="text-sm font-medium">Passphrase</label>
        <BaseInput
          id="passphrase"
          ref="passphraseInputRef"
          v-model="passphrase"
          type="password"
          placeholder="Enter your passphrase"
          required
          autocomplete="off"
          :disabled="loading"
        />
        <small class="text-xs text-muted"
          >Enter the passphrase to unlock your identity</small
        >
      </div>

      <BaseAlert v-if="error" variant="danger">{{ error }}</BaseAlert>

      <BaseButton variant="primary" type="submit" block :loading="loading">{{
        loading ? "Decrypting…" : "Unlock"
      }}</BaseButton>

      <BaseButton
        v-if="biometricUsable"
        variant="ghost"
        block
        @click="switchToBiometric"
      >
        Unlock with biometric
      </BaseButton>
    </form>
  </BaseModalShell>
</template>

<style scoped>
.close-x {
  position: absolute;
  top: -0.25rem;
  right: -0.25rem;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 2rem;
  height: 2rem;
  border-radius: var(--radius-sm, 0.25rem);
  color: var(--color-muted);
  transition: color 0.15s;
}
.close-x:active {
  color: var(--color-default);
}
@media (hover: hover) {
  .close-x:hover {
    color: var(--color-default);
  }
}
.help-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 1.5rem;
  height: 1.5rem;
  border-radius: var(--radius-sm, 0.25rem);
  color: var(--color-muted);
  transition: color 0.15s;
}
.help-btn:active,
.help-btn.active {
  color: var(--color-default);
}
@media (hover: hover) {
  .help-btn:hover {
    color: var(--color-default);
  }
}
</style>
