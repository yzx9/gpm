<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import type { AppLockError } from "@/api";
import { appUnlock, asAppLockError } from "@/api";
import { useAppLockState } from "@/composables";
import { LockKeyhole, ScanFace } from "@lucide/vue";
import { onMounted, ref } from "vue";
import BaseAlert from "./base/BaseAlert.vue";
import BaseButton from "./base/BaseButton.vue";
import BaseIcon from "./base/BaseIcon.vue";
import BaseModalShell from "./base/BaseModalShell.vue";

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
        // The at-rest master key is sealed by the biometric-gated Keystore key,
        // which Android destroyed when all enrolled biometrics were removed. The
        // master key is random (not passphrase-derived), so the store is
        // unrecoverable — and this overlay gates the whole app, so Settings is
        // unreachable. The only path is to wipe gpm at the OS level and set it
        // up again. (Uninstall also purges the stale Keystore aliases; "Clear
        // data" overwrites them on next setup — both work.)
        notice.value =
          "All fingerprints were removed, so gpm can no longer unlock your app key. Clear gpm's app data from Android Settings (or uninstall and reinstall) to set it up again.";
        break;
      default:
        notice.value = err.message || "Unlock failed";
    }
  } finally {
    setUnlockInFlight(false);
    loading.value = false;
  }
}

onMounted(() => {
  void tryUnlock();
});
</script>

<template>
  <BaseModalShell variant="center" :z="70" aria-label="App locked">
    <h1
      class="text-center text-display mb-1 flex items-center justify-center gap-2"
    >
      <BaseIcon :icon="LockKeyhole" :size="28" /> gpm
    </h1>
    <p class="text-center text-muted text-sm mb-6">App is locked</p>

    <BaseAlert v-if="notice" variant="danger" role="status" class="mb-4">
      {{ notice }}
    </BaseAlert>

    <BaseButton variant="primary" :loading="loading" @click="tryUnlock">
      <BaseIcon v-if="!loading" :icon="ScanFace" />
      <span>{{ loading ? "Unlocking…" : "Unlock with biometric" }}</span>
    </BaseButton>
  </BaseModalShell>
</template>
