<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { invoke } from "@tauri-apps/api/core";
import { appUnlock, asAppLockError } from "../appLock";
import { useAppLockState } from "../composables";
import type { AppLockError } from "../types";
import BaseButton from "./base/BaseButton.vue";
import BaseAlert from "./base/BaseAlert.vue";
import BaseModalShell from "./base/BaseModalShell.vue";

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
  <BaseModalShell variant="center" :z="70" aria-label="App locked">
    <h1 class="text-center text-display mb-1">🔐 gpm</h1>
    <p class="text-center text-muted text-sm mb-6">App is locked</p>

    <BaseAlert v-if="notice" variant="danger" role="status" class="mb-4">
      {{ notice }}
    </BaseAlert>

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
  </BaseModalShell>
</template>
