<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import type { BiometricError } from "@/api";
import {
  asBiometricError,
  biometricUnlock,
  disableBiometricUnlock,
  isBiometricAvailable,
  isBiometricUnlockEnabled,
  resetConfig,
  unlock,
} from "@/api";
import { LockKeyhole, ScanFace } from "@lucide/vue";
import { onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import BaseAlert from "./base/BaseAlert.vue";
import BaseButton from "./base/BaseButton.vue";
import BaseIcon from "./base/BaseIcon.vue";
import BaseInput from "./base/BaseInput.vue";
import BaseModalShell from "./base/BaseModalShell.vue";

const router = useRouter();

const passphrase = ref("");
const loading = ref(false);
const error = ref("");

// ── Biometric state ───────────────────────────────────────────────────
const biometricAvailable = ref(false);
const biometricEnabled = ref(false);
const biometricLoading = ref(false);
const biometricNotice = ref("");

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
        // User chose "Use passphrase" — keep the form, stay quiet.
        break;
      case "BIOMETRIC_KEY_INVALIDATED":
        biometricNotice.value =
          "Biometric was reset (new fingerprint?) — re-enable it in Settings.";
        await disableBiometricUnlock();
        biometricEnabled.value = false;
        break;
      case "WRONG_PASSPHRASE":
        biometricNotice.value =
          "Stored passphrase is invalid — re-enable biometric in Settings.";
        await disableBiometricUnlock();
        biometricEnabled.value = false;
        break;
      default:
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

async function onReset() {
  if (!confirm("Reset gpm? This will remove all local data and configuration."))
    return;
  try {
    await resetConfig();
    // The backend emits `identity-lock-state { locked: false }` on reset, which
    // closes this overlay. Then drop the user on Setup to reconfigure.
    router.push({ name: "setup" });
  } catch (e) {
    const appError = e as { message?: string };
    error.value = appError?.message || "Reset failed";
  }
}

onMounted(async () => {
  biometricAvailable.value = await isBiometricAvailable();
  biometricEnabled.value = await isBiometricUnlockEnabled();
  // Auto-prompt when enabled and available; cancel reveals the form silently.
  if (biometricEnabled.value && biometricAvailable.value) {
    await tryBiometricUnlock();
  }
});
</script>

<template>
  <BaseModalShell variant="center" :z="60" aria-label="Unlock identity">
    <h1
      class="text-center text-display mb-1 flex items-center justify-center gap-2"
    >
      <BaseIcon :icon="LockKeyhole" :size="28" /> gpm
    </h1>
    <p class="text-center text-muted text-sm mb-6">Identity is locked</p>

    <!-- Biometric notice (reset / stale / failure) -->
    <BaseAlert
      v-if="biometricNotice"
      variant="danger"
      role="status"
      class="mb-4"
    >
      {{ biometricNotice }}
    </BaseAlert>

    <!-- Unlock with biometric -->
    <BaseButton
      v-if="biometricAvailable && biometricEnabled"
      variant="secondary"
      :loading="biometricLoading"
      :disabled="loading"
      @click="tryBiometricUnlock"
    >
      <BaseIcon v-if="!biometricLoading" :icon="ScanFace" />
      <span>{{
        biometricLoading ? "Unlocking…" : "Unlock with biometric"
      }}</span>
    </BaseButton>

    <div
      v-if="biometricAvailable && biometricEnabled"
      class="flex items-center gap-2 my-4"
      aria-hidden="true"
    >
      <span class="divider-line"></span>
      <span class="text-xs text-subtle">or use passphrase</span>
      <span class="divider-line"></span>
    </div>

    <form @submit.prevent="onUnlock" class="flex flex-col gap-4">
      <div class="flex flex-col gap-1">
        <label for="passphrase" class="text-sm font-medium">Passphrase</label>
        <BaseInput
          id="passphrase"
          v-model="passphrase"
          type="password"
          placeholder="Enter your passphrase"
          required
          autocomplete="off"
          :disabled="loading"
          autofocus
        />
        <small class="text-xs text-muted"
          >Enter the passphrase to unlock your identity</small
        >
      </div>

      <BaseAlert v-if="error" variant="danger">{{ error }}</BaseAlert>

      <BaseButton variant="primary" type="submit" :loading="loading">{{
        loading ? "Decrypting…" : "Unlock"
      }}</BaseButton>

      <button
        type="button"
        class="self-center text-xs text-muted hover:text-danger transition-colors"
        @click="onReset"
      >
        Reset all data
      </button>
    </form>
  </BaseModalShell>
</template>

<style scoped>
.divider-line {
  flex: 1;
  height: 1px;
  background: var(--color-edge);
}
</style>
