<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { invoke } from "@tauri-apps/api/core";
import {
  asBiometricError,
  biometricUnlock,
  disableBiometricUnlock,
  isBiometricAvailable,
  isBiometricUnlockEnabled,
} from "../biometric";
import type { BiometricError } from "../types";

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
    await invoke("unlock", { passphrase: passphrase.value });
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
    await invoke("reset_config");
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
  <div
    class="overlay"
    role="dialog"
    aria-modal="true"
    aria-label="Unlock identity"
  >
    <div class="card">
      <h1 class="text-center text-display mb-1">🔐 gpm</h1>
      <p class="text-center text-muted text-sm mb-6">Identity is locked</p>

      <!-- Biometric notice (reset / stale / failure) -->
      <div
        v-if="biometricNotice"
        class="bg-danger-soft text-danger p-2 px-3 rounded-sm text-sm mb-4"
        role="status"
      >
        {{ biometricNotice }}
      </div>

      <!-- Unlock with biometric -->
      <button
        v-if="biometricAvailable && biometricEnabled"
        type="button"
        :disabled="biometricLoading || loading"
        class="btn-biometric"
        @click="tryBiometricUnlock"
      >
        <span v-if="biometricLoading" class="spinner" aria-hidden="true"></span>
        <span v-else>👁</span>
        <span>{{
          biometricLoading ? "Unlocking…" : "Unlock with biometric"
        }}</span>
      </button>

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
          <input
            id="passphrase"
            v-model="passphrase"
            type="password"
            placeholder="Enter your passphrase"
            required
            autocomplete="off"
            :disabled="loading"
            class="input-base"
            autofocus
          />
          <small class="text-xs text-muted"
            >Enter the passphrase to unlock your identity</small
          >
        </div>

        <div
          v-if="error"
          class="bg-danger-soft text-danger p-2 px-3 rounded-sm text-sm"
          role="alert"
        >
          {{ error }}
        </div>

        <button type="submit" :disabled="loading" class="btn-primary">
          <span v-if="loading" class="spinner-white" aria-hidden="true"></span>
          <span v-if="loading">Decrypting…</span>
          <span v-else>Unlock</span>
        </button>

        <button
          type="button"
          class="self-center text-xs text-muted hover:text-danger transition-colors"
          @click="onReset"
        >
          Reset all data
        </button>
      </form>
    </div>
  </div>
</template>

<style scoped>
.overlay {
  position: fixed;
  inset: 0;
  z-index: 60;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 1rem;
  /* Honor notch/gesture insets; the overlay sits above the safe-area-padded shell. */
  padding-top: calc(1rem + var(--safe-area-inset-top, 0px));
  padding-bottom: calc(1rem + var(--safe-area-inset-bottom, 0px));
  background: rgba(0, 0, 0, 0.4);
  /* The backdrop must fully capture interaction with the locked page behind it. */
  overscroll-behavior: contain;
}

.card {
  width: 100%;
  max-width: 420px;
  background: var(--color-surface);
  border-radius: var(--radius-lg);
  padding: 2rem;
  box-shadow: 0 2px 12px rgba(0, 0, 0, 0.08);
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

.btn-biometric {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 0.5rem;
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

.btn-biometric:hover:not(:disabled) {
  background: var(--color-hover);
}

.btn-biometric:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.divider-line {
  flex: 1;
  height: 1px;
  background: var(--color-edge);
}

.spinner {
  display: inline-block;
  width: 14px;
  height: 14px;
  border: 2px solid var(--color-edge);
  border-top-color: var(--color-accent);
  border-radius: 50%;
  animation: spin 0.6s linear infinite;
  vertical-align: middle;
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
</style>
