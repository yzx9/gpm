<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { exportSshPrivateKey, getSshPublicKey, type AppError } from "@/api";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseSpinner from "@/components/base/BaseSpinner.vue";
import { useLockState, useToast } from "@/composables";
import { navBack } from "@/utils/nav";
import {
  ArrowLeft,
  Copy,
  KeyRound,
  LockOpen,
  TriangleAlert,
} from "@lucide/vue";
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";

const { t } = useI18n();
const router = useRouter();
const { onLock } = useLockState();
const { toast } = useToast();

const publicKey = ref("");
const privateKey = ref("");
const showPrivate = ref(false);
const loading = ref(false);
const exporting = ref(false);
const error = ref("");

onMounted(loadPublicKey);

async function loadPublicKey() {
  loading.value = true;
  error.value = "";
  try {
    const result = await getSshPublicKey();
    publicKey.value = result.public_key;
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("sshKey.publicFailed");
  } finally {
    loading.value = false;
  }
}

async function exportPrivateKey() {
  if (!confirm(t("sshKey.exportConfirm"))) return;
  exporting.value = true;
  error.value = "";
  try {
    const result = await exportSshPrivateKey();
    privateKey.value = result.private_key;
    showPrivate.value = true;
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("sshKey.exportFailed");
  } finally {
    exporting.value = false;
  }
}

async function copyText(text: string) {
  try {
    await navigator.clipboard.writeText(text);
    toast.success(t("common.toast.copied"));
  } catch {
    toast.danger(t("common.toast.copyFailed"));
  }
}

// The unlock modal keeps this page mounted on auto-lock, so wipe any revealed
// private key the moment the identity locks (mirrors SettingsPage's onLock wipe).
onLock(() => {
  privateKey.value = "";
  showPrivate.value = false;
});

function goBack() {
  navBack(router, { name: "settings" });
}
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <header class="flex justify-between items-center mb-6" role="banner">
      <h1 class="text-xl flex items-center gap-1">
        <BaseIcon :icon="KeyRound" :size="24" /> {{ t("sshKey.title") }}
      </h1>
      <BaseButton size="sm" :aria-label="t('common.back')" @click="goBack">
        <BaseIcon :icon="ArrowLeft" /> {{ t("common.back") }}
      </BaseButton>
    </header>

    <BaseAlert v-if="error" variant="danger" class="mb-4">{{
      error
    }}</BaseAlert>

    <!-- Public key -->
    <section class="mb-6">
      <div class="flex justify-between items-center mb-2">
        <span class="text-xs text-muted">{{ t("sshKey.publicKeyLabel") }}</span>
        <button
          v-if="publicKey"
          class="btn-copy"
          :aria-label="t('sshKey.copy')"
          @click="copyText(publicKey)"
        >
          <BaseIcon :icon="Copy" /> {{ t("sshKey.copy") }}
        </button>
      </div>
      <div v-if="loading" class="flex items-center gap-2 text-muted py-4">
        <BaseSpinner />
      </div>
      <pre v-else class="key-display">{{ publicKey }}</pre>
    </section>

    <!-- Private key export -->
    <section>
      <BaseButton
        variant="action-danger"
        :loading="exporting"
        :disabled="showPrivate"
        @click="exportPrivateKey"
      >
        <BaseIcon :icon="LockOpen" /> {{ t("sshKey.exportPrivate") }}
      </BaseButton>

      <div v-if="showPrivate" class="mt-3 flex flex-col gap-2">
        <BaseAlert variant="danger">
          <BaseIcon
            :icon="TriangleAlert"
            :size="14"
            class="inline-block align-middle"
          />
          {{ t("sshKey.privateVisible") }}
        </BaseAlert>
        <div class="flex justify-end">
          <button class="btn-copy" @click="copyText(privateKey)">
            <BaseIcon :icon="Copy" /> {{ t("sshKey.copy") }}
          </button>
        </div>
        <pre class="key-display private-key-display">{{ privateKey }}</pre>
        <BaseButton
          variant="action"
          class="mt-1"
          @click="
            showPrivate = false;
            privateKey = '';
          "
        >
          {{ t("sshKey.hidePrivate") }}
        </BaseButton>
      </div>
    </section>
  </main>
</template>

<style scoped>
.key-display {
  padding: 0.6rem 0.75rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  background: var(--color-input);
  font-size: var(--text-xs);
  font-family: monospace;
  word-break: break-all;
  white-space: pre-wrap;
  max-height: 150px;
  overflow-y: auto;
  margin: 0;
}

.private-key-display {
  max-height: 300px;
}

.btn-copy {
  display: inline-flex;
  align-items: center;
  gap: 0.25rem;
  background: transparent;
  border: none;
  color: var(--color-accent);
  cursor: pointer;
  font-size: var(--text-sm);
}
</style>
