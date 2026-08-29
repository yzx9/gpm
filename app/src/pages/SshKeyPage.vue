<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<script setup lang="ts">
import {
  clearSshKey,
  exportSshPrivateKey,
  getConfig,
  getSshPublicKey,
  setSshKey,
  verifySshAuth,
  type AppError,
} from "@/api";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseHeader from "@/components/base/BaseHeader.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseSpinner from "@/components/base/BaseSpinner.vue";
import { isSshUrl as isSshRepoUrl } from "@/components/setup/url";
import SshKeyFields from "@/components/SshKeyFields.vue";
import { useActiveRepo, useDialog, useSecureClaim, useToast, useWipeOnLeave } from "@/composables";
import { optionalPassphrase } from "@/utils/passphrase";
import { Copy, LockOpen, Trash2, TriangleAlert } from "@lucide/vue";
import { computed, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";

const { t } = useI18n();
const { toast } = useToast();
const { dialog } = useDialog();
const router = useRouter();
const activeRepo = useActiveRepo();

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

// Replace/add form state. Gated on an SSH remote URL below — an SSH key
// cannot authenticate HTTPS, and a stored key takes precedence over a PAT,
// so offering it on an HTTPS repo would guide the user into a broken sync.
// Reads the sealed config URL; sync resolves origin from the clone's git
// config. The two cannot diverge until URL editing exists (R099 owns that
// dual write).
const repoUrl = ref("");
const newKey = ref("");
const newPassphrase = ref("");
const verifying = ref(false);
const replacing = ref(false);
const sshFields = ref<InstanceType<typeof SshKeyFields> | null>(null);
const canRotate = computed(() => isSshRepoUrl(repoUrl.value));
const busy = computed(
  () => exporting.value || removing.value || verifying.value || replacing.value,
);
const canReplace = computed(() => newKey.value.trim().length > 0 && !busy.value);

// Replace-form draft claim: the pasted/generated private key renders in the
// form itself, so hold a screen-capture claim while the draft is non-empty
// and drop it once the form empties (saved, cleared, or wiped on lock/leave
// — all of which write the refs this watches). Best-effort, unlike the
// export path's fail-closed abort: a failed acquire can't un-render the
// user's own typing.
const { acquire: acquireFormClaim, release: releaseFormClaim } = useSecureClaim();
let formClaimHeld = false;
watch([newKey, newPassphrase], ([key, passphrase]) => {
  const dirty = key !== "" || passphrase !== "";
  if (dirty === formClaimHeld) return;
  formClaimHeld = dirty;
  if (dirty) {
    void acquireFormClaim().then((ok) => {
      // The draft may have cleared while the acquire was in flight — drop
      // the late claim instead of stranding FLAG_SECURE (release no-ops
      // when nothing was actually acquired).
      if (!ok || !formClaimHeld) releaseFormClaim();
    });
  } else {
    releaseFormClaim();
  }
});

onMounted(() => {
  void loadPublicKey();
  void loadRepoUrl();
});

async function loadPublicKey() {
  loading.value = true;
  error.value = "";
  try {
    const repoId = await activeRepo.currentId();
    const result = await getSshPublicKey(repoId);
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

async function loadRepoUrl() {
  try {
    const config = await getConfig();
    repoUrl.value = config.url;
  } catch (e) {
    // A failed load surfaces as the page error and leaves the replace section
    // hidden (an empty URL classifies as non-SSH) — the public-key half of
    // the page still works.
    const appError = e as AppError;
    error.value = appError?.message || t("sshKey.loadFailed");
  }
}

async function replaceKey() {
  const key = newKey.value.trim();
  if (!key || busy.value) return;
  error.value = "";
  const passphrase = optionalPassphrase(newPassphrase.value);
  // Resolve the target repository once and reuse it for both halves —
  // two independent currentId() calls could straddle an active-repo switch.
  let repoId: string;
  try {
    repoId = await activeRepo.currentId();
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("sshKey.replaceFailed");
    return;
  }
  verifying.value = true;
  try {
    // Probe the candidate pair against the remote before saving it — refuse
    // to replace a working key with one that can't authenticate.
    await verifySshAuth(repoId, key, passphrase);
  } catch (e) {
    verifying.value = false;
    const appError = e as AppError;
    error.value = appError?.message
      ? `${t("sshKey.verifyFailed")} ${appError.message}`
      : t("sshKey.verifyFailed");
    return;
  }
  verifying.value = false;
  replacing.value = true;
  try {
    await setSshKey(repoId, key, passphrase);
    newKey.value = "";
    newPassphrase.value = "";
    sshFields.value?.reset();
    toast.success(t("sshKey.replacedToast"));
    // Refresh the public half (and clear noKey when this was an add).
    await loadPublicKey();
  } catch (e) {
    const appError = e as AppError;
    error.value =
      appError?.code === "CONFIG_BUSY"
        ? t("settings.auth.configBusy")
        : appError?.message || t("sshKey.replaceFailed");
  } finally {
    replacing.value = false;
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
    const repoId = await activeRepo.currentId();
    const claimed = await withClaim(() => exportSshPrivateKey(repoId));
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
    // Explicit target, matching replaceKey — remove and replace on one page
    // must not silently diverge onto different repositories.
    const repoId = await activeRepo.currentId();
    await clearSshKey(repoId);
    toast.success(t("sshKey.removedToast"));
    // The key is gone — return to the settings card, which re-derives the active
    // method (a stored PAT becomes active, else None).
    router.push({ name: "settingsRepository" });
  } catch (e) {
    const appError = e as AppError;
    error.value =
      appError?.code === "CONFIG_BUSY"
        ? t("settings.auth.configBusy")
        : appError?.message || t("sshKey.removeFailed");
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

// Wipe any revealed private key and the replace-form drafts on either lock
// (identity hard lock or app-gate re-lock), on browser back, and on unmount —
// matching the other secret pages. The unlock modal can keep this page mounted
// on auto-lock, so unmount alone can't guarantee a wipe. Returns whether
// user-authored drafts were dropped, for the post-unlock "drafts cleared"
// toast.
function wipeSensitive(): boolean {
  hidePrivate();
  const dirty = newKey.value !== "" || newPassphrase.value !== "";
  newKey.value = "";
  newPassphrase.value = "";
  sshFields.value?.reset();
  return dirty;
}

useWipeOnLeave(wipeSensitive);
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <BaseHeader :back-fallback="{ name: 'settings' }" :title="t('sshKey.title')" />

    <BaseAlert v-if="error" variant="danger" class="mb-4">{{ error }}</BaseAlert>

    <!-- No SSH key configured — a normal state, not an error. -->
    <BaseAlert v-if="noKey" variant="info" class="mb-4">{{ t("sshKey.noKey") }}</BaseAlert>

    <template v-if="!noKey">
      <!-- Public key -->
      <section class="mb-6">
        <div class="flex justify-between items-center mb-2">
          <span class="text-xs text-muted">{{ t("sshKey.publicKeyLabel") }}</span>
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
            <BaseIcon :icon="TriangleAlert" :size="14" class="inline-block align-middle" />
            {{ t("sshKey.privateVisible") }}
          </BaseAlert>
          <div class="flex justify-end">
            <BaseButton variant="link" size="xs" tone="accent" @click="copyText(privateKey)">
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
        <BaseButton variant="action-danger" :loading="removing" @click="removeKey">
          <BaseIcon :icon="Trash2" /> {{ t("sshKey.remove") }}
        </BaseButton>
      </section>
    </template>

    <!-- Replace / add key — outside the noKey wrapper so it doubles as the
         re-add path after a removal. SSH remote URLs only (see canRotate). -->
    <section v-if="canRotate" class="mt-6">
      <h2 class="text-sm font-medium mb-2">{{ t("sshKey.replaceLabel") }}</h2>
      <SshKeyFields
        ref="sshFields"
        v-model:ssh-key="newKey"
        v-model:ssh-passphrase="newPassphrase"
        v-model:error="error"
        :disabled="busy"
      />
      <BaseButton
        variant="action"
        class="mt-2"
        :loading="verifying || replacing"
        :disabled="!canReplace"
        @click="replaceKey"
      >
        {{ verifying ? t("sshKey.verifying") : noKey ? t("sshKey.add") : t("sshKey.replace") }}
      </BaseButton>
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
</style>
