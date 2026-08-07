<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<script setup lang="ts">
import type { AppLockError } from "@/api";
import { appUnlock, asAppLockError } from "@/api";
import { useAppLockState, useDiagnosticsExport, Z } from "@/composables";
import { reconcileLocaleFromBackend } from "@/i18n";
import { appLockUnlockPrompt } from "@/i18n/native";
import { LockKeyhole, ScanFace } from "@lucide/vue";
import { nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import BaseAlert from "./base/BaseAlert.vue";
import BaseButton from "./base/BaseButton.vue";
import BaseIcon from "./base/BaseIcon.vue";
import BaseModalShell from "./base/BaseModalShell.vue";

const { setUnlockInFlight, shouldAutoPrompt } = useAppLockState();
const { exporting, runExport } = useDiagnosticsExport();

const { t } = useI18n();

const loading = ref(false);
const notice = ref("");
// Template ref on the primary button so we can move focus to it on mount — the
// shell is aria-modal, so landing keyboard/AT users on Unlock (the only action)
// beats leaving focus on whatever sat behind the opaque gate.
const unlockButton = ref<InstanceType<typeof BaseButton> | null>(null);

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
      case "KEYSTORE_CANCELLED":
        // User dismissed the prompt — keep the overlay, offer a retry.
        break;
      case "KEYSTORE_KEY_INVALIDATED":
        // The seal master key is sealed by the biometric-gated Keystore key,
        // which Android destroyed when all enrolled biometrics were removed. The
        // master key is random (not passphrase-derived), so the store is
        // unrecoverable — and this overlay gates the whole app, so Settings is
        // unreachable. The only path is to wipe gpm at the OS level and set it
        // up again. (Uninstall also purges the stale Keystore aliases; "Clear
        // data" overwrites them on next setup — both work.)
        notice.value = t("common.appLock.keyInvalidatedNotice");
        break;
      case "KEYSTORE_UNAVAILABLE":
        // Sensor temporarily unusable (hw busy/unavailable). Distinct from
        // KEY_INVALIDATED (biometrics removed = unrecoverable): this is usually
        // transient, so point at retry rather than the dead-end generic message.
        notice.value = t("common.appLock.biometricUnavailable");
        break;
      default:
        notice.value = err.message || t("common.appLock.unlockFailed");
    }
  } finally {
    setUnlockInFlight(false);
    loading.value = false;
  }
}

// Move focus to Unlock (the only action) for keyboard/AT users. Called on mount
// for an idle re-lock (auto-prompt suppressed, button enabled) and again via the
// `loading` watch below once a biometric attempt ends with the overlay still up.
// On the cold-start/resume path tryUnlock flips loading=true synchronously in
// onMounted before this runs, disabling the button and no-op'ing the mount-time
// focus — so the watch's loading→false is what actually lands focus there.
function focusUnlock() {
  void nextTick().then(() => {
    (
      unlockButton.value?.$el as HTMLButtonElement | null | undefined
    )?.focus?.();
  });
}

onMounted(() => {
  console.info("[gpm:ui] app-lock overlay shown");
  focusUnlock();
  // Auto-fire the biometric prompt only for a cold start / resume re-lock. An
  // idle re-lock suppresses it (the user is present but idle) — they tap the
  // button below. R057.
  if (shouldAutoPrompt.value) {
    void tryUnlock();
  }
});

// Refocus Unlock after a biometric attempt resolves (cancel/failure) while the
// overlay is still mounted. On a successful unlock the overlay unmounts right
// after, so the extra focus is harmless.
watch(loading, (now, prev) => {
  if (prev && !now) focusUnlock();
});

onUnmounted(() => {
  console.info("[gpm:ui] app-lock overlay closed");
});
</script>

<template>
  <BaseModalShell
    variant="fullscreen"
    :z="Z.gate"
    :dismiss-on-backdrop="false"
    :dismiss-on-back="false"
    :aria-label="t('common.appLock.title')"
  >
    <!-- `.lock-body` is flex:1 inside the shell's fullscreen column, so this
         group is vertically centered while `.lock-foot` pins to the bottom
         safe-area. -->
    <div class="lock-body">
      <h1 class="lock-title">
        <BaseIcon :icon="LockKeyhole" :size="28" /> gpm
      </h1>
      <p class="lock-subtitle">{{ t("common.appLock.locked") }}</p>

      <BaseAlert
        v-if="notice"
        variant="danger"
        role="status"
        class="lock-notice"
      >
        {{ notice }}
      </BaseAlert>

      <!-- Cross-disable with the diagnostics link: Unlock is disabled while an
           export is in flight (exporting), just as the link is disabled while a
           biometric prompt is up (loading), so the system BiometricPrompt and the
           SAF picker / confirm never stack. -->
      <BaseButton
        ref="unlockButton"
        variant="primary"
        block
        :loading="loading"
        :disabled="exporting"
        class="lock-unlock"
        @click="tryUnlock"
      >
        <BaseIcon v-if="!loading" :icon="ScanFace" />
        <span>{{
          loading
            ? t("common.appLock.unlocking")
            : t("common.appLock.unlockWithBiometric")
        }}</span>
      </BaseButton>
    </div>

    <div class="lock-foot">
      <!-- Discreet, subordinate to Unlock: a muted text link. Disabled while a
           biometric prompt is in flight (loading) so the in-WebView confirm
           never stacks under the system BiometricPrompt. runExport passes
           z:Z.gate so its confirm/toast surface above this opaque gate. -->
      <BaseButton
        variant="link"
        tone="muted"
        size="sm"
        :loading="exporting"
        :disabled="loading"
        :aria-label="t('common.appLock.diagnostics')"
        @click="() => runExport({ z: Z.gate })"
      >
        {{ t("common.appLock.diagnostics") }}
      </BaseButton>
    </div>
  </BaseModalShell>
</template>

<style scoped>
.lock-body {
  flex: 1;
  display: flex;
  flex-direction: column;
  justify-content: center;
  align-items: stretch;
}
.lock-title {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 0.5rem;
  font-size: var(--text-display);
  font-weight: 600;
  color: var(--color-default);
  margin: 0 0 0.25rem;
}
.lock-subtitle {
  text-align: center;
  color: var(--color-muted);
  font-size: var(--text-sm);
  margin: 0 0 1.5rem;
}
.lock-notice {
  margin: 0 0 1rem;
}
.lock-unlock {
  margin-top: 0.25rem;
}
.lock-foot {
  padding-top: 1rem;
  display: flex;
  justify-content: center;
}
</style>
