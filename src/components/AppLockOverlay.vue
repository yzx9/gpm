<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { invoke } from "@tauri-apps/api/core";
import { appUnlock, asAppLockError } from "../appLock";
import { useAppLockState } from "../utils/useAppLockState";
import type { AppLockError } from "../types";
import BaseButton from "./base/BaseButton.vue";

const router = useRouter();
const { setUnlockInFlight } = useAppLockState();

const loading = ref(false);
const notice = ref("");

async function tryUnlock() {
  // Re-entry guard: the overlay auto-prompts on mount, and the user can also
  // tap the button. Don't stack a second biometric prompt (the backend's
  // idempotency check runs before the prompt's await, so two concurrent calls
  // would both reach BiometricPrompt and one would error).
  if (loading.value) return;
  notice.value = "";
  loading.value = true;
  // Loop guard: suppress the resume re-lock while the biometric prompt is up.
  setUnlockInFlight(true);
  try {
    await appUnlock();
    // Success: the backend emits `app-lock-state { locked: false }`, which
    // useAppLockState mirrors and App.vue's `v-if` reacts to, unmounting this
    // overlay. Nothing to do here.
  } catch (e) {
    const err = asAppLockError(e) as AppLockError;
    switch (err.code) {
      case "BIOMETRIC_CANCELLED":
        // User dismissed the prompt — keep the overlay, offer a retry.
        break;
      case "BIOMETRIC_KEY_INVALIDATED":
        notice.value =
          "Biometric was reset (all fingerprints removed?) — the store can no longer be unlocked. Reset to reconfigure.";
        break;
      default:
        notice.value = err.message || "Unlock failed";
    }
  } finally {
    setUnlockInFlight(false);
    loading.value = false;
  }
}

async function onReset() {
  if (!confirm("Reset gpm? This will remove all local data and configuration."))
    return;
  try {
    await invoke("reset_config");
    router.push({ name: "setup" });
  } catch (e) {
    const err = e as { message?: string };
    notice.value = err?.message || "Reset failed";
  }
}

onMounted(() => {
  void tryUnlock();
});
</script>

<template>
  <div class="overlay" role="dialog" aria-modal="true" aria-label="App locked">
    <div class="card">
      <h1 class="text-center text-display mb-1">🔐 gpm</h1>
      <p class="text-center text-muted text-sm mb-6">App is locked</p>

      <div
        v-if="notice"
        class="bg-danger-soft text-danger p-2 px-3 rounded-sm text-sm mb-4"
        role="status"
      >
        {{ notice }}
      </div>

      <BaseButton variant="primary" :loading="loading" @click="tryUnlock">
        <span v-if="!loading">👁</span>
        <span>{{ loading ? "Unlocking…" : "Unlock with biometric" }}</span>
      </BaseButton>

      <button
        type="button"
        class="self-center text-xs text-muted hover:text-danger transition-colors mt-6"
        @click="onReset"
      >
        Reset all data
      </button>
    </div>
  </div>
</template>

<style scoped>
.overlay {
  position: fixed;
  inset: 0;
  z-index: 70;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 1rem;
  /* Honor notch/gesture insets; the overlay sits above the safe-area-padded shell
     and above the identity UnlockModal (z-index 60). */
  padding-top: calc(1rem + var(--safe-area-inset-top, 0px));
  padding-bottom: calc(1rem + var(--safe-area-inset-bottom, 0px));
  background: rgba(0, 0, 0, 0.4);
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
</style>
