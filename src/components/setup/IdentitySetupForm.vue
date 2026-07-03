<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import type { AppError } from "@/api";
import {
  clearPendingIdentity,
  completeSetup,
  completeSetupFromFile,
  listRecipients,
  pickIdentityFile,
  validateIdentity,
  verifyPickedIdentity,
  type PickedIdentityResult,
  type RecipientInfo,
} from "@/api";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import BaseTextarea from "@/components/base/BaseTextarea.vue";
import { ArrowLeft, FileText, KeyRound, TriangleAlert } from "@lucide/vue";
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { truncateKey } from "./url";

const props = defineProps<{
  /** Step-1 SSH key, so the "Use my SSH key for decryption" affordance has
   *  something to copy into `identity`. */
  sshKey?: string;
  /** Whether step 1 was an SSH URL — together with sshKey + hasSshRecipients
   *  this gates the reuse affordance. */
  isSshUrl?: boolean;
}>();

const emit = defineEmits<{
  done: [];
  back: [];
}>();

// Step-2 state — verbatim from the original SetupPage.
const recipients = ref<RecipientInfo[]>([]);
const selectedRecipient = ref("");
const identity = ref("");
const passphrase = ref("");
const identityType = ref<string>("");
const isIdentityEncrypted = ref(false);
const loadingRecipients = ref(false);
const loadingIdentity = ref(false);
const identitySource = ref<"paste" | "file">("paste");
const pickedFile = ref<PickedIdentityResult | null>(null);
const picking = ref(false);
const verifying = ref(false);
const error = ref("");

const hasSshRecipients = computed(() =>
  recipients.value.some(
    (r) => r.key_type === "ssh_ed25519" || r.key_type === "ssh_rsa",
  ),
);

const canReuseSshKey = computed(
  () =>
    props.isSshUrl === true &&
    hasSshRecipients.value &&
    !!props.sshKey &&
    !!props.sshKey.trim(),
);

const isSshIdentity = computed(
  () =>
    identityType.value === "ssh_ed25519" || identityType.value === "ssh_rsa",
);

async function fetchRecipients() {
  loadingRecipients.value = true;
  try {
    recipients.value = await listRecipients();
    // Auto-select first recipient if only one exists
    if (recipients.value.length === 1) {
      selectedRecipient.value = recipients.value[0].public_key;
    }
  } catch {
    // Recipients may not exist (empty repo) — that's fine
    recipients.value = [];
  } finally {
    loadingRecipients.value = false;
  }
}

function validateStep2(): string | null {
  if (identitySource.value === "file") {
    if (!pickedFile.value) return "No identity file selected";
    if (!pickedFile.value.recipient) return "Unlock the identity file first";
    if (recipients.value.length > 0 && !selectedRecipient.value)
      return "Please select a recipient";
    return null;
  }
  if (!identity.value.trim()) return "Age identity is required";
  const trimmed = identity.value.trim();
  const isAgeKey = trimmed.startsWith("AGE-SECRET-KEY-");
  const isSshKey =
    trimmed.startsWith("-----BEGIN OPENSSH PRIVATE KEY-----") ||
    trimmed.startsWith("-----BEGIN RSA PRIVATE KEY-----");
  if (!isAgeKey && !isSshKey)
    return "Identity must be an age key (AGE-SECRET-KEY-...) or SSH private key";
  if (recipients.value.length > 0 && !selectedRecipient.value)
    return "Please select a recipient";
  if (isSshIdentity.value && isIdentityEncrypted.value && !passphrase.value)
    return "SSH key passphrase is required";
  return null;
}

async function onCompleteSetup() {
  error.value = "";
  const validationError = validateStep2();
  if (validationError) {
    error.value = validationError;
    return;
  }

  loadingIdentity.value = true;
  try {
    if (identitySource.value === "file") {
      await completeSetupFromFile(passphrase.value || null);
    } else {
      await completeSetup(identity.value, passphrase.value || null);
    }
    emit("done");
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Setup failed";
  } finally {
    loadingIdentity.value = false;
  }
}

async function onPickFile() {
  picking.value = true;
  error.value = "";
  try {
    const info = await pickIdentityFile();
    pickedFile.value = info;
    identityType.value = info.key_type;
    isIdentityEncrypted.value = info.encrypted;
    passphrase.value = "";
    identitySource.value = "file";
    identity.value = ""; // watch is guarded in file mode
  } catch (e) {
    const appError = e as AppError;
    // CANCELLED just means the user dismissed the picker — not an error.
    if (appError?.code !== "CANCELLED") {
      error.value = appError?.message || "Failed to read identity file";
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
    const res = await verifyPickedIdentity(passphrase.value);
    if (pickedFile.value) pickedFile.value.recipient = res.recipient;
  } catch (e) {
    const appError = e as AppError;
    // The backend abandoned the file on failure — drop it and return to paste.
    error.value =
      appError?.code === "WRONG_PASSPHRASE"
        ? "Wrong passphrase — the file was discarded"
        : appError?.message || "Verification failed";
    onUsePaste();
  } finally {
    verifying.value = false;
  }
}

function onUsePaste() {
  identitySource.value = "paste";
  pickedFile.value = null;
  identityType.value = "";
  isIdentityEncrypted.value = false;
  passphrase.value = "";
  identity.value = "";
  clearPendingIdentity().catch(() => {});
}

function clearPendingFile() {
  if (identitySource.value === "file") {
    clearPendingIdentity().catch(() => {});
  }
}

function useSshKeyForIdentity() {
  identity.value = props.sshKey ?? "";
}

function goBack() {
  error.value = "";
  clearPendingFile();
  emit("back");
}

onMounted(fetchRecipients);
onUnmounted(clearPendingFile);

// Detect identity type and SSH-key encryption status when identity changes
watch(identity, async (val) => {
  if (identitySource.value === "file") return;
  const trimmed = val.trim();
  if (trimmed.startsWith("AGE-SECRET-KEY-PQ-1")) {
    identityType.value = "post_quantum";
    isIdentityEncrypted.value = false;
    return;
  }
  if (trimmed.startsWith("AGE-SECRET-KEY-")) {
    identityType.value = "x25519";
    isIdentityEncrypted.value = false;
    return;
  }
  if (
    !trimmed.startsWith("-----BEGIN OPENSSH PRIVATE KEY-----") &&
    !trimmed.startsWith("-----BEGIN RSA PRIVATE KEY-----")
  ) {
    identityType.value = "";
    isIdentityEncrypted.value = false;
    return;
  }
  try {
    const info = await validateIdentity(trimmed);
    identityType.value = info.key_type;
    isIdentityEncrypted.value = info.encrypted;
  } catch {
    identityType.value = "";
    isIdentityEncrypted.value = false;
  }
});
</script>

<template>
  <form class="flex flex-col gap-4" @submit.prevent="onCompleteSetup">
    <!-- Back button — MUST be the first button[type='button'] in this form
         (the back-navigation test relies on this ordering). -->
    <button
      type="button"
      class="self-start text-sm text-muted hover:text-accent transition-colors inline-flex items-center gap-1"
      @click="goBack"
    >
      <BaseIcon :icon="ArrowLeft" /> Back
    </button>

    <h2 class="text-lg font-semibold">Select Recipient</h2>
    <p class="text-xs text-muted">
      This repository encrypts secrets to the following recipients. Select yours
      and paste the matching identity key.
    </p>

    <!-- Recipients list -->
    <div v-if="loadingRecipients" class="text-center py-4 text-sm text-muted">
      Loading recipients…
    </div>

    <div v-else-if="recipients.length > 0" class="flex flex-col gap-2">
      <div
        v-for="r in recipients"
        :key="r.public_key"
        :class="[
          'flex items-start gap-3 p-3 rounded-[var(--radius-md)] border cursor-pointer transition-colors',
          selectedRecipient === r.public_key
            ? 'border-accent bg-accent-soft'
            : 'border-[var(--color-edge)] bg-[var(--color-input)] hover:bg-hover',
        ]"
        @click="selectedRecipient = r.public_key"
      >
        <input
          type="radio"
          :checked="selectedRecipient === r.public_key"
          class="mt-0.5 accent-[var(--color-accent)]"
          tabindex="-1"
          readonly
        />
        <div class="flex flex-col gap-0.5 min-w-0">
          <div class="flex items-center gap-1.5">
            <code class="text-xs font-mono break-all">{{
              truncateKey(r.public_key)
            }}</code>
            <span
              v-if="r.key_type !== 'x25519'"
              class="shrink-0 text-[10px] font-medium px-1.5 py-0.5 rounded bg-[var(--color-edge)] text-muted"
              >{{
                r.key_type === "post_quantum"
                  ? "PQ"
                  : r.key_type === "plugin"
                    ? "Plugin"
                    : "SSH"
              }}</span
            >
          </div>
          <span v-if="r.comment" class="text-xs text-muted">{{
            r.comment
          }}</span>
        </div>
      </div>
    </div>

    <div
      v-else
      class="text-sm text-muted p-3 bg-[var(--color-input)] rounded-[var(--radius-md)]"
    >
      No recipients file found. You can still provide your identity.
    </div>

    <!-- SSH key reuse offer -->
    <BaseButton
      v-if="canReuseSshKey && !identity.trim() && identitySource === 'paste'"
      variant="secondary"
      size="sm"
      @click="useSshKeyForIdentity"
    >
      <BaseIcon :icon="KeyRound" /> Use my SSH key for decryption
    </BaseButton>

    <div class="flex flex-col gap-1">
      <label for="identity" class="text-sm font-medium">Age Identity</label>
      <BaseTextarea
        id="identity"
        v-model="identity"
        rows="5"
        placeholder="AGE-SECRET-KEY-...&#10;or paste an SSH private key"
        autocomplete="off"
        spellcheck="false"
        :disabled="loadingIdentity || identitySource === 'file'"
      />

      <!-- Picked-file panel: the bytes live in backend state, not here -->
      <div
        v-if="identitySource === 'file' && pickedFile"
        class="flex flex-col gap-2 text-xs bg-[var(--color-input)] border border-[var(--color-edge)] rounded-[var(--radius-md)] p-2 px-2.5"
      >
        <div class="flex items-center justify-between gap-2">
          <span class="min-w-0 truncate">
            <BaseIcon
              :icon="FileText"
              :size="14"
              class="inline-block align-middle shrink-0"
            />
            {{ pickedFile.filename || "identity file" }} ·
            {{ pickedFile.key_type
            }}<span v-if="pickedFile.encrypted"> · encrypted</span>
          </span>
          <button
            type="button"
            class="shrink-0 text-muted hover:text-danger transition-colors"
            @click="onUsePaste"
          >
            Remove
          </button>
        </div>

        <!-- Public key, once usable (unencrypted, or unlocked) -->
        <div v-if="pickedFile.recipient" class="flex flex-col gap-0.5">
          <span class="text-muted">Public key</span>
          <code class="font-mono break-all">{{
            truncateKey(pickedFile.recipient)
          }}</code>
        </div>

        <!-- Encrypted: unlock + verify before the key is usable -->
        <div v-else class="flex flex-col gap-1">
          <BaseInput
            v-model="passphrase"
            type="password"
            placeholder="Passphrase to unlock this file"
            autocomplete="off"
            :disabled="verifying"
          />
          <BaseButton
            variant="secondary"
            :disabled="verifying || !passphrase"
            @click="onVerify"
          >
            {{ verifying ? "Verifying…" : "Unlock & verify" }}
          </BaseButton>
          <small class="text-muted"
            >Enter the file's passphrase to verify it and reveal its public key.
            A wrong passphrase discards the file.</small
          >
        </div>
      </div>
      <small v-else class="text-xs text-muted"
        >Paste your age secret key (AGE-SECRET-KEY-...) or SSH private
        key</small
      >

      <!-- Upload via the native picker (hidden once a file is picked) -->
      <BaseButton
        v-if="identitySource !== 'file'"
        variant="secondary"
        size="sm"
        :disabled="picking || loadingIdentity"
        @click="onPickFile"
      >
        {{ picking ? "Reading…" : "📁 Upload identity file…" }}
      </BaseButton>
    </div>

    <!-- SSH key passphrase (paste path: required for an encrypted SSH key) -->
    <div
      v-if="identitySource === 'paste' && isSshIdentity && isIdentityEncrypted"
      class="flex flex-col gap-1"
    >
      <label for="passphrase" class="text-sm font-medium"
        >SSH Key Passphrase</label
      >
      <BaseInput
        id="passphrase"
        v-model="passphrase"
        type="password"
        placeholder="Passphrase to decrypt the SSH key"
        autocomplete="off"
        :disabled="loadingIdentity"
      />
      <small class="text-xs text-muted"
        >This SSH key is passphrase-encrypted. Enter its passphrase to use it as
        an age identity.</small
      >
    </div>

    <!-- Optional at-rest encryption (paste path; x25519 keys only) -->
    <div
      v-else-if="identitySource === 'paste' && identityType === 'x25519'"
      class="flex flex-col gap-1"
    >
      <label for="passphrase" class="text-sm font-medium"
        >Passphrase (optional)</label
      >
      <BaseInput
        id="passphrase"
        v-model="passphrase"
        type="password"
        placeholder="Leave empty for plaintext storage"
        autocomplete="new-password"
        :disabled="loadingIdentity"
      />
      <small class="text-xs text-muted"
        >Encrypts the identity file at rest. Recommended for Android.</small
      >
      <BaseAlert v-if="!passphrase.trim()" variant="warning">
        <BaseIcon
          :icon="TriangleAlert"
          :size="14"
          class="inline-block align-middle"
        />
        Without a passphrase, the identity is stored in plaintext.
      </BaseAlert>
    </div>

    <BaseAlert variant="info" class="text-center">
      Stored locally. Nothing leaves your device.
    </BaseAlert>

    <BaseAlert v-if="error" variant="danger">{{ error }}</BaseAlert>

    <BaseButton variant="primary" type="submit" :loading="loadingIdentity">{{
      loadingIdentity ? "Verifying…" : "Complete Setup"
    }}</BaseButton>
  </form>
</template>
