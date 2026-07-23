<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import type { AppLockError } from "@/api";
import { appUnlock, asAppLockError } from "@/api";
import { useAppLockState } from "@/composables";
import { reconcileLocaleFromBackend } from "@/i18n";
import { appLockUnlockPrompt } from "@/i18n/native";
import { LockKeyhole, ScanFace } from "@lucide/vue";
import { onMounted, onUnmounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import BaseAlert from "./base/BaseAlert.vue";
import BaseButton from "./base/BaseButton.vue";
import BaseIcon from "./base/BaseIcon.vue";
import BaseModalShell from "./base/BaseModalShell.vue";

const { setUnlockInFlight } = useAppLockState();

const { t } = useI18n();

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
    // Authoritative locale before building prompt text: the boot locale is the
    // system-locale guess (injected pre-paint), so a user who pinned a different
    // language would otherwise get this cold-start prompt in the system locale.
    // This overlay auto-prompts on mount, so it can't rely on main.ts's reconcile
    // (fire-and-forget) to have completed first. Idempotent when already matched.
    await reconcileLocaleFromBackend();
    await appUnlock(appLockUnlockPrompt());
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
        // The seal master key is sealed by the biometric-gated Keystore key,
        // which Android destroyed when all enrolled biometrics were removed. The
        // master key is random (not passphrase-derived), so the store is
        // unrecoverable — and this overlay gates the whole app, so Settings is
        // unreachable. The only path is to wipe gpm at the OS level and set it
        // up again. (Uninstall also purges the stale Keystore aliases; "Clear
        // data" overwrites them on next setup — both work.)
        notice.value = t("common.appLock.keyInvalidatedNotice");
        break;
      default:
        notice.value = err.message || t("common.appLock.unlockFailed");
    }
  } finally {
    setUnlockInFlight(false);
    loading.value = false;
  }
}

onMounted(() => {
  console.info("[gpm:ui] app-lock overlay shown");
  void tryUnlock();
});

onUnmounted(() => {
  console.info("[gpm:ui] app-lock overlay closed");
});
</script>

<template>
  <BaseModalShell
    variant="center"
    :z="70"
    :aria-label="t('common.appLock.title')"
  >
    <h1
      class="text-center text-display mb-1 flex items-center justify-center gap-2"
    >
      <BaseIcon :icon="LockKeyhole" :size="28" /> gpm
    </h1>
    <p class="text-center text-muted text-sm mb-6">
      {{ t("common.appLock.locked") }}
    </p>

    <BaseAlert v-if="notice" variant="danger" role="status" class="mb-4">
      {{ notice }}
    </BaseAlert>

    <BaseButton variant="primary" block :loading="loading" @click="tryUnlock">
      <BaseIcon v-if="!loading" :icon="ScanFace" />
      <span>{{
        loading
          ? t("common.appLock.unlocking")
          : t("common.appLock.unlockWithBiometric")
      }}</span>
    </BaseButton>
  </BaseModalShell>
</template>
