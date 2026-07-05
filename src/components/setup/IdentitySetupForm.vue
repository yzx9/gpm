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
  verifyPastedIdentity,
  verifyPickedIdentity,
  type PickedIdentityResult,
  type RecipientInfo,
} from "@/api";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import BaseTextarea from "@/components/base/BaseTextarea.vue";
import PassphraseField from "@/components/PassphraseField.vue";
import PassphraseUnrecoverableAck from "@/components/PassphraseUnrecoverableAck.vue";
import {
  ArrowLeft,
  Check,
  FileText,
  KeyRound,
  TriangleAlert,
} from "@lucide/vue";
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { truncateKey } from "./url";

const props = defineProps<{
  /** Step-1 SSH key, so the "Use my SSH key for decryption" affordance has
   *  something to copy into `identity`. */
  sshKey?: string;
  /** Whether step 1 was an SSH URL — together with sshKey + hasSshRecipients
   *  this gates the reuse affordance. */
  isSshUrl?: boolean;
}>();

const emit = defineEmits<{ done: []; back: [] }>();

// Recipients are read-only context now; the match is derived from the identity,
// not selected by hand. `derivedRecipient` is the single source of truth for
// "we have a public key to match against the list".
const recipients = ref<RecipientInfo[]>([]);
const derivedRecipient = ref<string | null>(null);

const identity = ref("");
const passphrase = ref("");
const identityType = ref<string>("");
const isIdentityEncrypted = ref(false);
const malformedIdentity = ref(false);
const loadingRecipients = ref(false);
const loadingIdentity = ref(false);
const identitySource = ref<"paste" | "file">("paste");
const pickedFile = ref<PickedIdentityResult | null>(null);
const picking = ref(false);
const verifying = ref(false);
const deriving = ref(false);
const error = ref("");
// Confirm-field controller for the x25519 at-rest passphrase (validate/reset).
const pf = ref<InstanceType<typeof PassphraseField> | null>(null);
// Forced "this x25519 at-rest passphrase cannot be recovered" ack. SSH keys
// and the file/verify paths are out of scope. Reset whenever the passphrase OR
// the pasted identity changes (watches below) so a stale ack can't carry
// across contexts or be reused for a different committed value.
const ackX25519 = ref(false);
// Required only on the paste+x25519 at-rest path, and only once a passphrase is
// actually typed (empty optional = plaintext = no lockout risk).
const ackRequired = computed(
  () =>
    identitySource.value === "paste" &&
    identityType.value === "x25519" &&
    !!passphrase.value &&
    !ackX25519.value,
);
// Ack is value-bound: any edit to the passphrase invalidates it.
watch(
  () => passphrase.value,
  () => {
    ackX25519.value = false;
  },
);

// Race guard: a monotonic token so a stale validate_identity response (from an
// earlier paste) cannot overwrite a newer edit's derived recipient.
const deriveSeq = ref(0);

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

const matchedRecipient = computed(() =>
  derivedRecipient.value
    ? (recipients.value.find((r) => r.public_key === derivedRecipient.value) ??
      null)
    : null,
);

const matchStatus = computed<
  "none" | "deriving" | "match" | "noMatch" | "neutral"
>(() => {
  if (recipients.value.length === 0) return "none";
  if (deriving.value || verifying.value) return "deriving";
  if (derivedRecipient.value === null) return "neutral";
  return matchedRecipient.value ? "match" : "noMatch";
});

const statusAlert = computed<{
  variant: "success" | "warning" | "info";
  text: string;
} | null>(() => {
  switch (matchStatus.value) {
    case "none":
      return null;
    case "match":
      return {
        variant: "success",
        text: "✓ This identity matches a recipient in this repository.",
      };
    case "noMatch":
      return {
        variant: "warning",
        text: "This identity doesn't match any recipient in the repository. Use a matching key, or ask the repo admin to add yours.",
      };
    case "deriving":
      return { variant: "info", text: "Deriving public key…" };
    case "neutral":
      // Only prompt when there's an action to take (encrypted SSH paste,
      // pre-verify). For partial typing or the file path, stay quiet.
      if (
        identitySource.value === "paste" &&
        isSshIdentity.value &&
        isIdentityEncrypted.value
      ) {
        return {
          variant: "info",
          text: "Verify your SSH key to confirm it matches.",
        };
      }
      return null;
  }
  return null;
});

async function fetchRecipients() {
  loadingRecipients.value = true;
  try {
    // Guard `?? []`: production always returns a Vec, but a mocked `invoke`
    // (or a future shape change) returning undefined must not crash the render.
    recipients.value = (await listRecipients()) ?? [];
  } catch {
    // Recipients may not exist (empty repo) — that's fine.
    recipients.value = [];
  } finally {
    loadingRecipients.value = false;
  }
}

function validateStep2(): string | null {
  if (identitySource.value === "file") {
    if (!pickedFile.value) return "No identity file selected";
    if (!pickedFile.value.recipient) return "Unlock the identity file first";
  } else {
    if (!identity.value.trim()) return "Age identity is required";
    const trimmed = identity.value.trim();
    const isAgeKey = trimmed.startsWith("AGE-SECRET-KEY-");
    const isSshKey =
      trimmed.startsWith("-----BEGIN OPENSSH PRIVATE KEY-----") ||
      trimmed.startsWith("-----BEGIN RSA PRIVATE KEY-----");
    if (!isAgeKey && !isSshKey)
      return "Identity must be an age key (AGE-SECRET-KEY-...) or SSH private key";
    if (malformedIdentity.value)
      return "This doesn't look like a valid age or SSH key.";
    if (isSshIdentity.value && isIdentityEncrypted.value && !passphrase.value)
      return "SSH key passphrase is required";
    if (
      isSshIdentity.value &&
      isIdentityEncrypted.value &&
      !derivedRecipient.value
    )
      return "Verify your SSH key first";
    // x25519 at-rest passphrase confirmation (pf is null on SSH/file paths —
    // those enter an existing passphrase, not a new one to confirm).
    const passphraseError = pf.value?.validate() ?? null;
    if (passphraseError) return passphraseError;
    if (ackRequired.value) {
      return "Please acknowledge that this passphrase cannot be recovered.";
    }
  }
  // Last check (mirrors the Store::save_identity backstop): hard-block a
  // derived recipient that matches nothing in a non-empty repo.
  if (
    recipients.value.length > 0 &&
    derivedRecipient.value &&
    !matchedRecipient.value
  )
    return "This identity doesn't match any recipient in the repository. Use a matching key, or ask the repo admin to add yours.";
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
    derivedRecipient.value = info.recipient;
    malformedIdentity.value = false;
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
    derivedRecipient.value = res.recipient;
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

async function onVerifyPaste() {
  if (!passphrase.value || !identity.value.trim()) return;
  verifying.value = true;
  error.value = "";
  try {
    const res = await verifyPastedIdentity(
      identity.value.trim(),
      passphrase.value,
    );
    derivedRecipient.value = res.recipient;
  } catch (e) {
    const appError = e as AppError;
    error.value =
      appError?.code === "WRONG_PASSPHRASE"
        ? "Wrong passphrase"
        : appError?.message || "Verification failed";
    derivedRecipient.value = null;
  } finally {
    verifying.value = false;
  }
}

function onUsePaste() {
  identitySource.value = "paste";
  pickedFile.value = null;
  identityType.value = "";
  isIdentityEncrypted.value = false;
  derivedRecipient.value = null;
  malformedIdentity.value = false;
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

/** Backend derivation for a pasted identity, with a race guard. Sets
 *  `derivedRecipient` (null for encrypted SSH awaiting Verify). */
async function deriveRecipient(identityText: string) {
  const seq = ++deriveSeq.value;
  deriving.value = true;
  try {
    const info = await validateIdentity(identityText);
    if (seq !== deriveSeq.value) return; // stale — a newer edit superseded us
    identityType.value = info.key_type;
    isIdentityEncrypted.value = info.encrypted;
    derivedRecipient.value = info.recipient;
    malformedIdentity.value = false;
  } catch {
    if (seq !== deriveSeq.value) return;
    identityType.value = "";
    isIdentityEncrypted.value = false;
    derivedRecipient.value = null;
    malformedIdentity.value = true;
  } finally {
    if (seq === deriveSeq.value) deriving.value = false;
  }
}

// Detect identity type and derive the recipient when the pasted identity looks
// complete. The unconditional top-of-watch reset is what guarantees a stale
// match can never survive an edit.
watch(identity, (val) => {
  if (identitySource.value === "file") return;
  // Clear stale state first, every time. Clears `error` too so a stale verify
  // error ("Wrong passphrase") doesn't linger after the user edits the identity.
  derivedRecipient.value = null;
  deriving.value = false;
  malformedIdentity.value = false;
  error.value = "";
  // A different identity is a different ack context — force a re-ack.
  ackX25519.value = false;

  const trimmed = val.trim();
  if (!trimmed) {
    identityType.value = "";
    isIdentityEncrypted.value = false;
    return;
  }
  if (trimmed.startsWith("AGE-SECRET-KEY-PQ-1")) {
    identityType.value = "post_quantum";
    isIdentityEncrypted.value = false;
    return;
  }
  if (trimmed.startsWith("AGE-PLUGIN-")) {
    identityType.value = "plugin";
    isIdentityEncrypted.value = false;
    return;
  }
  if (trimmed.startsWith("AGE-SECRET-KEY-")) {
    identityType.value = "x25519";
    isIdentityEncrypted.value = false;
    // Completeness gate: a full key line (prefix + ≥1 non-space). Do NOT
    // hardcode the bech32 length — let the backend parse decide validity.
    if (/^AGE-SECRET-KEY-1\S/m.test(trimmed)) void deriveRecipient(trimmed);
    return;
  }
  const isSsh =
    trimmed.startsWith("-----BEGIN OPENSSH PRIVATE KEY-----") ||
    trimmed.startsWith("-----BEGIN RSA PRIVATE KEY-----");
  if (isSsh) {
    // Type + encryption need a backend parse; gate on the END marker so we
    // don't fire mid-paste.
    if (/-----END (OPENSSH|RSA) PRIVATE KEY-----/.test(trimmed))
      void deriveRecipient(trimmed);
    return;
  }
  identityType.value = "";
  isIdentityEncrypted.value = false;
});

// Scroll the matched recipient into view (long team lists).
watch(matchedRecipient, (m) => {
  if (!m) return;
  void nextTick(() => {
    // Escape the attribute-value interpolation so a public_key containing `"`
    // or `\` (none in age/SSH keys today, but defensive against future formats)
    // can't break out of the selector.
    const pubkey = m.public_key.replace(/[\\"]/g, "\\$&");
    document
      .querySelector(`[data-pubkey="${pubkey}"]`)
      ?.scrollIntoView({ block: "nearest" });
  });
});

onMounted(fetchRecipients);
onUnmounted(clearPendingFile);
</script>

<template>
  <form class="flex flex-col gap-4" @submit.prevent="onCompleteSetup">
    <!-- Back button — MUST be the first button[type='button'] in this form
         (the back-navigation test relies on this ordering). -->
    <button
      type="button"
      class="self-start text-sm text-muted hover:text-accent active:text-accent transition-colors inline-flex items-center gap-1"
      @click="goBack"
    >
      <BaseIcon :icon="ArrowLeft" /> Back
    </button>

    <h2 class="text-lg font-semibold">Recipients in this repository</h2>
    <p class="text-xs text-muted">
      This repository encrypts secrets to the recipients below. Paste the
      identity that matches one of them.
    </p>

    <!-- Recipients list (read-only context; the match is derived, not selected) -->
    <div v-if="loadingRecipients" class="text-center py-4 text-sm text-muted">
      Loading recipients…
    </div>

    <BaseAlert v-else-if="recipients.length === 0" variant="info">
      This is a fresh repository with no recipients yet. Paste your identity —
      it will be the first.
    </BaseAlert>

    <div v-else class="flex flex-col gap-2 max-h-56 overflow-y-auto">
      <div
        v-for="r in recipients"
        :key="r.public_key"
        :data-pubkey="r.public_key"
        :aria-current="
          matchedRecipient?.public_key === r.public_key ? 'true' : undefined
        "
        :class="[
          'flex items-start gap-3 p-3 rounded-[var(--radius-md)] border transition-colors',
          matchedRecipient?.public_key === r.public_key
            ? 'border-accent bg-accent-soft'
            : 'border-[var(--color-edge)] bg-[var(--color-input)]',
        ]"
      >
        <BaseIcon
          v-if="matchedRecipient?.public_key === r.public_key"
          :icon="Check"
          :size="16"
          class="mt-0.5 shrink-0 text-accent"
        />
        <div class="flex flex-col gap-0.5 min-w-0">
          <div class="flex items-center gap-1.5 flex-wrap">
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
            <span
              v-if="matchedRecipient?.public_key === r.public_key"
              class="shrink-0 text-[10px] font-medium px-1.5 py-0.5 rounded bg-accent text-white"
              >your key</span
            >
          </div>
          <span v-if="r.comment" class="text-xs text-muted">{{
            r.comment
          }}</span>
        </div>
      </div>
    </div>

    <!-- Single, mutually-exclusive status alert. aria-live so AT announces
         match / no-match when the derived recipient changes. -->
    <div aria-live="polite">
      <BaseAlert v-if="statusAlert" :variant="statusAlert.variant">{{
        statusAlert.text
      }}</BaseAlert>
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
            class="shrink-0 text-muted hover:text-danger active:text-danger transition-colors"
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

    <!-- Inline unsupported / malformed alerts for the pasted key. -->
    <BaseAlert v-if="identityType === 'post_quantum'" variant="warning">
      <BaseIcon
        :icon="TriangleAlert"
        :size="14"
        class="inline-block align-middle"
      />
      Post-quantum age keys aren't supported yet.
    </BaseAlert>
    <BaseAlert v-else-if="identityType === 'plugin'" variant="warning">
      <BaseIcon
        :icon="TriangleAlert"
        :size="14"
        class="inline-block align-middle"
      />
      age-plugin identities aren't supported for decryption yet.
    </BaseAlert>
    <BaseAlert v-else-if="malformedIdentity" variant="danger">
      <BaseIcon
        :icon="TriangleAlert"
        :size="14"
        class="inline-block align-middle"
      />
      This doesn't look like a valid age or SSH key.
    </BaseAlert>

    <!-- SSH key passphrase + Verify (paste path: required for an encrypted SSH key) -->
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
        :disabled="loadingIdentity || verifying"
      />
      <BaseButton
        variant="secondary"
        size="sm"
        :disabled="verifying || !passphrase"
        @click="onVerifyPaste"
      >
        <BaseIcon :icon="KeyRound" />
        {{ verifying ? "Verifying…" : "Verify" }}
      </BaseButton>
      <small class="text-xs text-muted"
        >Enter the SSH key's passphrase and verify to confirm it matches a
        recipient.</small
      >
    </div>

    <!-- Optional at-rest encryption (paste path; x25519 keys only) -->
    <PassphraseField
      v-else-if="identitySource === 'paste' && identityType === 'x25519'"
      ref="pf"
      id="identity-passphrase"
      v-model="passphrase"
      label="Passphrase (optional)"
      placeholder="Leave empty for plaintext storage"
      :optional="true"
      :disabled="loadingIdentity"
    >
      <template #help>
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
      </template>
    </PassphraseField>

    <!-- x25519 at-rest: forced unrecoverable ack (only once a passphrase is
         typed; empty = plaintext = no lockout risk). The SSH-key passphrase
         field above decrypts an existing key rather than setting a new at-rest
         passphrase, so it gets no ack. -->
    <PassphraseUnrecoverableAck
      v-if="
        identitySource === 'paste' && identityType === 'x25519' && passphrase
      "
      v-model="ackX25519"
    />

    <BaseAlert variant="info" class="text-center">
      Stored locally. Nothing leaves your device.
    </BaseAlert>

    <BaseAlert v-if="error" variant="danger">{{ error }}</BaseAlert>

    <BaseButton
      variant="primary"
      type="submit"
      :loading="loadingIdentity"
      :disabled="ackRequired"
      >{{ loadingIdentity ? "Verifying…" : "Complete Setup" }}</BaseButton
    >
  </form>
</template>
