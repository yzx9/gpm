<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<script setup lang="ts">
import {
  clearSshKey,
  exportSshPrivateKey,
  getSshPublicKey,
  type AppError,
} from "@/api";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseHeader from "@/components/base/BaseHeader.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseSpinner from "@/components/base/BaseSpinner.vue";
import {
  useDialog,
  useSecureClaim,
  useToast,
  useWipeOnLeave,
} from "@/composables";
import { Copy, KeyRound, LockOpen, Trash2, TriangleAlert } from "@lucide/vue";
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";

const { t } = useI18n();
const { toast } = useToast();
const { dialog } = useDialog();
const router = useRouter();

const publicKey = ref("");
const privateKey = ref("");
const showPrivate = ref(false);
const loading = ref(false);
const exporting = ref(false);
const removing = ref(false);
const error = ref("");
/** No SSH key configured — a normal state, surfaced as an empty page (not an
 * error). Reached if the key was removed after the settings card linked here. */
const noKey = ref(false);
// R031: hold a screen-capture claim while the private key is on screen.
// `withClaim` raises FLAG_SECURE before it arrives; `release` drops it on hide /
// lock / unmount (onScopeDispose backs up the unmount path).
const { withClaim, release: releaseSecure } = useSecureClaim();

onMounted(loadPublicKey);

async function loadPublicKey() {
  loading.value = true;
  error.value = "";
  try {
    const result = await getSshPublicKey();
    if (result.public_key === null) {
      noKey.value = true;
      publicKey.value = "";
    } else {
      noKey.value = false;
      publicKey.value = result.public_key;
    }
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("sshKey.publicFailed");
  } finally {
    loading.value = false;
  }
}

async function exportPrivateKey() {
  const confirmed = await dialog.confirm({
    message: t("sshKey.exportConfirm"),
    confirmLabel: t("common.button.export"),
    danger: true,
  });
  if (!confirmed) return;
  exporting.value = true;
  error.value = "";
  try {
    // withClaim raises FLAG_SECURE before the private key arrives; a failed
    // acquire returns null → abort (the per-op replacement for the route abort).
    const claimed = await withClaim(() => exportSshPrivateKey());
    if (!claimed) {
      error.value = t("common.toast.secureScreenFailed");
      return;
    }
    privateKey.value = claimed.private_key;
    showPrivate.value = true;
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("sshKey.exportFailed");
  } finally {
    exporting.value = false;
  }
}

async function removeKey() {
  const confirmed = await dialog.confirm({
    message: t("sshKey.removeConfirm"),
    confirmLabel: t("common.button.remove"),
    danger: true,
  });
  if (!confirmed) return;
  removing.value = true;
  error.value = "";
  try {
    await clearSshKey();
    toast.success(t("sshKey.removedToast"));
    // The key is gone — return to the settings card, which re-derives the active
    // method (a stored PAT becomes active, else None).
    router.push({ name: "settingsRepository" });
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("sshKey.removeFailed");
  } finally {
    removing.value = false;
  }
}

/** Hide the private key and drop the screen-capture claim. */
function hidePrivate() {
  privateKey.value = "";
  showPrivate.value = false;
  releaseSecure();
}

async function copyText(text: string) {
  try {
    await navigator.clipboard.writeText(text);
    toast.success(t("common.toast.copied"));
  } catch {
    toast.danger(t("common.toast.copyFailed"));
  }
}

// Wipe any revealed private key on a hard identity lock, on browser back, and on
// unmount — matching the other secret pages (useWipeOnLeave covers all three,
// including the lock the old onLock wired). The unlock modal can keep this page
// mounted on auto-lock, so unmount alone can't guarantee a wipe.
useWipeOnLeave(hidePrivate);
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <BaseHeader
      :back-fallback="{ name: 'settings' }"
      :title="t('sshKey.title')"
      :title-icon="KeyRound"
    />

    <BaseAlert v-if="error" variant="danger" class="mb-4">{{
      error
    }}</BaseAlert>

    <!-- No SSH key configured — a normal state, not an error. -->
    <BaseAlert v-if="noKey" variant="info" class="mb-4">{{
      t("sshKey.noKey")
    }}</BaseAlert>

    <template v-if="!noKey">
      <!-- Public key -->
      <section class="mb-6">
        <div class="flex justify-between items-center mb-2">
          <span class="text-xs text-muted">{{
            t("sshKey.publicKeyLabel")
          }}</span>
          <BaseButton
            v-if="publicKey"
            variant="link"
            size="xs"
            tone="accent"
            :aria-label="t('sshKey.copy')"
            @click="copyText(publicKey)"
          >
            <BaseIcon :icon="Copy" /> {{ t("sshKey.copy") }}
          </BaseButton>
        </div>
        <div v-if="loading" class="flex items-center gap-2 text-muted py-4">
          <BaseSpinner />
        </div>
        <pre v-else class="key-display">{{ publicKey }}</pre>
      </section>

      <!-- Private key export -->
      <section class="mb-6">
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
            <BaseButton
              variant="link"
              size="xs"
              tone="accent"
              @click="copyText(privateKey)"
            >
              <BaseIcon :icon="Copy" /> {{ t("sshKey.copy") }}
            </BaseButton>
          </div>
          <pre class="key-display private-key-display">{{ privateKey }}</pre>
          <BaseButton variant="action" class="mt-1" @click="hidePrivate">
            {{ t("sshKey.hidePrivate") }}
          </BaseButton>
        </div>
      </section>

      <!-- Remove key (method switching: clears SSH so a stored PAT / None takes over) -->
      <section>
        <BaseButton
          variant="action-danger"
          :loading="removing"
          @click="removeKey"
        >
          <BaseIcon :icon="Trash2" /> {{ t("sshKey.remove") }}
        </BaseButton>
      </section>
    </template>
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
</style>
