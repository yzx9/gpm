<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<script setup lang="ts">
import type { AppError } from "@/api";
import {
  clearPendingIdentity,
  completeSetupFromFile,
  createGpgStore,
  isConfigured,
  pickIdentityFile,
  pushRepo,
  verifyPickedIdentity,
  type PickedIdentityResult,
} from "@/api";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import { resolveActiveRepoId, useWipeOnLeave } from "@/composables";
import { CircleCheck, FileText, KeyRound } from "@lucide/vue";
import { computed, onUnmounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import RepoAuthFields from "./RepoAuthFields.vue";
import { isSshUrl as isSshRepoUrl } from "./url";

const { t } = useI18n();

const emit = defineEmits<{ done: [] }>();

// The picked GPG key's public metadata (bytes stay backend-side). `gpgVerified`
// gates Create on a successful S2K verify.
const pickedFile = ref<PickedIdentityResult | null>(null);
const gpgVerified = ref(false);
const passphrase = ref("");

// Optional remote (local-first: a remote is not required to create).
const repoUrl = ref("");
const pat = ref("");
const sshKey = ref("");
const sshPassphrase = ref("");

const picking = ref(false);
const verifying = ref(false);
const loading = ref(false);
const error = ref("");

const isSshUrl = computed(() => isSshRepoUrl(repoUrl.value));

// Drop any staged identity if the user leaves without completing (no-op after a
// successful complete_setup_from_file, which consumes it).
onUnmounted(() => {
  clearPendingIdentity().catch(() => {});
});

// Wipe the typed S2K passphrase + optional git credentials on browser back /
// unmount. The secret key itself never enters the WebView (only its public
// metadata does, which is left out of the wipe). No lock wiring: no identity
// exists during setup.
useWipeOnLeave(
  () => {
    passphrase.value = "";
    pat.value = "";
    sshKey.value = "";
    sshPassphrase.value = "";
  },
  { lock: false },
);

async function onPickFile() {
  picking.value = true;
  error.value = "";
  try {
    const info = await pickIdentityFile();
    // Only a GPG key is meaningful here; a non-GPG pick is a mismatch.
    if (info.key_type !== "gpg") {
      error.value = t("setup.createGpg.err.errImport");
      pickedFile.value = null;
      return;
    }
    pickedFile.value = info;
    gpgVerified.value = false;
    passphrase.value = "";
  } catch (e) {
    const appError = e as AppError;
    // CANCELLED just means the user dismissed the picker — not an error.
    if (appError?.code !== "CANCELLED") {
      error.value = appError?.message || t("setup.createGpg.err.errImport");
    }
  } finally {
    picking.value = false;
  }
}

async function onVerify() {
  if (!passphrase.value) return;
  verifying.value = true;
  error.value = "";
  try {
    await verifyPickedIdentity(passphrase.value);
    gpgVerified.value = true;
  } catch (e) {
    const appError = e as AppError;
    // The backend abandoned the file on failure — drop it so it can't be saved.
    error.value =
      appError?.code === "WRONG_PASSPHRASE"
        ? t("setup.identity.err.errWrongPassFile")
        : appError?.message || t("setup.identity.err.errVerifyFailed");
    pickedFile.value = null;
    gpgVerified.value = false;
    passphrase.value = "";
    clearPendingIdentity().catch(() => {});
  } finally {
    verifying.value = false;
  }
}

function removeFile() {
  pickedFile.value = null;
  gpgVerified.value = false;
  passphrase.value = "";
  clearPendingIdentity().catch(() => {});
}

function validate(): string | null {
  if (!pickedFile.value) return t("setup.createGpg.validation.errImportFirst");
  if (!gpgVerified.value) return t("setup.createGpg.validation.errVerifyFirst");

  const url = repoUrl.value.trim();
  const hasAuth = Boolean(pat.value.trim() || sshKey.value.trim());
  if (!url && hasAuth) {
    return t("setup.create.validation.errUrlOrClearAuth");
  }
  if (url) {
    const isHttps = url.startsWith("https://");
    const isSsh = isSshRepoUrl(url);
    if (!isHttps && !isSsh) {
      return t("setup.create.validation.errUrlFormat");
    }
    if (isSsh && !sshKey.value.trim()) {
      return t("setup.create.validation.errSshKeyRequired");
    }
  }
  return null;
}

async function onCreate() {
  error.value = "";
  const validationError = validate();
  if (validationError) {
    error.value = validationError;
    return;
  }

  loading.value = true;
  try {
    const hasRemote = repoUrl.value.trim().length > 0;
    // A store that's already configured (e.g. retrying after a non-fatal push
    // failure) must NOT be re-bootstrapped: create_gpg_store clears config +
    // rm -rf's the repo, and the staged identity is already consumed, so a
    // re-run would destroy the saved identity and strand the store. When the
    // store is complete, skip straight to the (retry) push.
    const configured = await isConfigured();
    if (!configured) {
      // create_gpg_store seeds .gpg-id + .public-keys/<token> + gopass's init
      // commits. It does NOT push (deferred until the identity is durable) and
      // does NOT consume the staged identity (complete_setup_from_file does).
      await createGpgStore(
        hasRemote ? repoUrl.value.trim() : null,
        hasRemote && !isSshUrl.value ? pat.value || null : null,
        hasRemote && isSshUrl.value ? sshKey.value : null,
        hasRemote && isSshUrl.value ? sshPassphrase.value || null : null,
      );

      // GPG stores the S2K-locked armor byte-unchanged — no seal passphrase
      // (storage_passphrase=None). The S2K passphrase was verified above.
      await completeSetupFromFile(null);
    }

    if (hasRemote) {
      // First push — or, after a prior push failure, the retry. A failed push
      // blocks navigation so the user sees it rather than silently believing the
      // store synced.
      try {
        await pushRepo(await resolveActiveRepoId());
      } catch (e) {
        const pushError = e as AppError;
        error.value =
          (pushError?.message || t("setup.create.err.errPush")) +
          t("setup.create.err.errPushSuffix");
        console.warn("[setup] push failed", e);
        return;
      }
    }

    emit("done");
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("setup.createGpg.err.errCreate");
    console.warn("[setup] gpg create failed", e);
  } finally {
    loading.value = false;
  }
}
</script>

<template>
  <form @submit.prevent="onCreate" class="flex flex-col gap-4">
    <h2 class="text-lg font-semibold">{{ t("setup.createGpg.heading") }}</h2>
    <p class="text-xs text-muted">{{ t("setup.createGpg.intro") }}</p>
    <p class="text-xs text-muted">{{ t("common.setup.introAppKey") }}</p>

    <!-- Import an existing GPG secret key. Bytes live in backend state; only
         public metadata (uid/fingerprint/membership) is shown. -->
    <div class="flex flex-col gap-2">
      <BaseButton
        v-if="!pickedFile"
        variant="secondary"
        :loading="picking"
        :disabled="loading"
        @click="onPickFile"
      >
        <BaseIcon :icon="KeyRound" />
        {{
          picking
            ? t("setup.createGpg.importLoading")
            : t("setup.createGpg.importButton")
        }}
      </BaseButton>

      <div
        v-else
        class="flex flex-col gap-2 text-xs bg-input border border-edge rounded-md p-2 px-2.5"
      >
        <div class="flex items-center justify-between gap-2">
          <span class="min-w-0 truncate inline-flex items-center gap-1">
            <BaseIcon :icon="FileText" :size="14" class="shrink-0" />
            <span class="truncate">{{
              pickedFile.filename || t("setup.identity.fileTypeFallback")
            }}</span>
            <span
              class="shrink-0 text-[10px] font-medium px-1.5 py-0.5 rounded bg-edge text-muted"
              >{{ t("setup.identity.badgeGpg") }}</span
            >
          </span>
          <BaseButton
            variant="link"
            size="xs"
            tone="danger"
            class="shrink-0"
            :disabled="loading"
            @click="removeFile"
          >
            {{ t("setup.identity.fileRemove") }}
          </BaseButton>
        </div>

        <div v-if="pickedFile.user_id" class="font-medium break-all">
          {{ pickedFile.user_id }}
        </div>
        <div v-if="pickedFile.fingerprint" class="flex flex-col gap-0.5">
          <span class="text-muted">{{
            t("setup.identity.gpgFingerprint")
          }}</span>
          <code class="font-mono break-all">{{ pickedFile.fingerprint }}</code>
        </div>

        <!-- Membership badge: for a fresh create the probe is null (no store
             yet to match against) — neither badge renders, which is correct. -->
        <span
          v-if="pickedFile.is_recipient === true"
          class="shrink-0 self-start text-[10px] font-medium px-1.5 py-0.5 rounded bg-accent text-on-accent"
          >{{ t("setup.identity.gpgRecipient") }}</span
        >
        <span
          v-else-if="pickedFile.is_recipient === false"
          class="shrink-0 self-start text-[10px] font-medium px-1.5 py-0.5 rounded bg-edge text-muted"
          >{{ t("setup.identity.gpgNotRecipient") }}</span
        >

        <!-- S2K passphrase verify (required before Create). -->
        <div v-if="!gpgVerified" class="flex flex-col gap-1">
          <BaseInput
            id="gpg-verify-passphrase"
            v-model="passphrase"
            type="password"
            :placeholder="t('setup.identity.gpgPassphrasePlaceholder')"
            autocomplete="off"
            :disabled="verifying || loading"
          />
          <BaseButton
            variant="secondary"
            :disabled="verifying || loading || !passphrase"
            @click="onVerify"
          >
            <BaseIcon :icon="KeyRound" />
            {{
              verifying
                ? t("setup.identity.gpgVerifyLoading")
                : t("setup.identity.gpgVerifyButton")
            }}
          </BaseButton>
        </div>
        <div v-else class="inline-flex items-center gap-1 text-success">
          <BaseIcon :icon="CircleCheck" :size="14" />
          {{ t("setup.identity.gpgVerified") }}
        </div>
      </div>
    </div>

    <!-- Optional remote -->
    <div class="flex flex-col gap-3 pt-4 border-t border-edge">
      <div>
        <span class="text-sm font-medium">{{
          t("setup.create.remoteLabel")
        }}</span>
        <p class="text-xs text-muted">{{ t("setup.create.remoteHint") }}</p>
      </div>
      <div class="flex flex-col gap-4">
        <RepoAuthFields
          v-model:repo-url="repoUrl"
          v-model:pat="pat"
          v-model:ssh-key="sshKey"
          v-model:ssh-passphrase="sshPassphrase"
          :show-keygen="false"
          :url-required="false"
          :disabled="loading"
        />
      </div>
    </div>

    <BaseAlert variant="info" class="text-center">
      {{ t("common.setup.storedLocally") }}
    </BaseAlert>

    <BaseAlert v-if="error" variant="danger">{{ error }}</BaseAlert>

    <BaseButton variant="primary" type="submit" :loading="loading">{{
      loading ? t("setup.create.creating") : t("setup.create.buttonCreate")
    }}</BaseButton>
  </form>
</template>
