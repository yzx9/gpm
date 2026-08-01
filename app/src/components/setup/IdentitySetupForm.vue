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
import { useWipeOnLeave } from "@/composables";
import {
  ArrowLeft,
  Check,
  FileText,
  KeyRound,
  TriangleAlert,
} from "@lucide/vue";
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { truncateKey } from "./url";

const { t } = useI18n();

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
// Confirm-field controller for the x25519 seal passphrase (validate/reset).
const pf = ref<InstanceType<typeof PassphraseField> | null>(null);
// Forced "this x25519 seal passphrase cannot be recovered" ack. SSH keys
// and the file/verify paths are out of scope. Reset whenever the passphrase OR
// the pasted identity changes (watches below) so a stale ack can't carry
// across contexts or be reused for a different committed value.
const ackX25519 = ref(false);
// Required only on the paste+x25519 seal path, and only once a passphrase is
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
        text: t("setup.identity.status.match"),
      };
    case "noMatch":
      return {
        variant: "warning",
        text: t("setup.identity.noMatchWarning"),
      };
    case "deriving":
      return { variant: "info", text: t("setup.identity.status.deriving") };
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
          text: t("setup.identity.status.verifySshHint"),
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
    if (!pickedFile.value) return t("setup.identity.validation.errNoFile");
    if (!pickedFile.value.recipient)
      return t("setup.identity.validation.errUnlockFileFirst");
  } else {
    if (!identity.value.trim())
      return t("setup.identity.validation.errIdentityRequired");
    const trimmed = identity.value.trim();
    const isAgeKey = trimmed.startsWith("AGE-SECRET-KEY-");
    const isSshKey =
      trimmed.startsWith("-----BEGIN OPENSSH PRIVATE KEY-----") ||
      trimmed.startsWith("-----BEGIN RSA PRIVATE KEY-----");
    if (!isAgeKey && !isSshKey)
      return t("setup.identity.validation.errIdentityFormat");
    if (malformedIdentity.value) return t("setup.identity.malformed");
    if (isSshIdentity.value && isIdentityEncrypted.value && !passphrase.value)
      return t("setup.identity.validation.errSshPassRequired");
    if (
      isSshIdentity.value &&
      isIdentityEncrypted.value &&
      !derivedRecipient.value
    )
      return t("setup.identity.validation.errVerifyFirst");
    // x25519 seal passphrase confirmation (pf is null on SSH/file paths —
    // those enter an existing passphrase, not a new one to confirm).
    const passphraseError = pf.value?.validate() ?? null;
    if (passphraseError) return passphraseError;
    if (ackRequired.value) {
      return t("setup.identity.validation.errAckRequired");
    }
  }
  // Last check (mirrors the Store::save_identity backstop): hard-block a
  // derived recipient that matches nothing in a non-empty repo.
  if (
    recipients.value.length > 0 &&
    derivedRecipient.value &&
    !matchedRecipient.value
  )
    return t("setup.identity.noMatchWarning");
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
    error.value = appError?.message || t("setup.identity.err.errSetup");
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
      error.value = appError?.message || t("setup.identity.err.errReadFile");
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
        ? t("setup.identity.err.errWrongPassFile")
        : appError?.message || t("setup.identity.err.errVerifyFailed");
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
        ? t("setup.identity.err.errWrongPass")
        : appError?.message || t("setup.identity.err.errVerifyFailed");
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

// Wipe the pasted master identity + passphrase on browser back and on unmount
// (step 2→1 unmounts this form) so the most sensitive frontend string isn't
// left for GC. No lock wiring: no identity exists during setup.
useWipeOnLeave(
  () => {
    identity.value = "";
    passphrase.value = "";
    pf.value?.reset();
  },
  { lock: false },
);

onMounted(fetchRecipients);
onUnmounted(clearPendingFile);
</script>

<template>
  <form class="flex flex-col gap-4" @submit.prevent="onCompleteSetup">
    <!-- Back button — MUST be the first button[type='button'] in this form
         (the back-navigation test relies on this ordering). BaseButton renders
         <button type="button"> by default, preserving that. -->
    <BaseButton variant="ghost" class="self-start" @click="goBack">
      <BaseIcon :icon="ArrowLeft" /> {{ t("common.back") }}
    </BaseButton>

    <h2 class="text-lg font-semibold">{{ t("setup.identity.heading") }}</h2>
    <p class="text-xs text-muted">{{ t("setup.identity.intro") }}</p>
    <p class="text-xs text-muted">{{ t("common.setup.introAppKey") }}</p>

    <!-- Recipients list (read-only context; the match is derived, not selected) -->
    <div v-if="loadingRecipients" class="text-center py-4 text-sm text-muted">
      {{ t("setup.identity.loadingRecipients") }}
    </div>

    <BaseAlert v-else-if="recipients.length === 0" variant="info">
      {{ t("setup.identity.noRecipientsAlert") }}
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
          'flex items-start gap-3 p-3 rounded-md border transition-colors',
          matchedRecipient?.public_key === r.public_key
            ? 'border-accent bg-accent-soft'
            : 'border-edge bg-input',
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
              class="shrink-0 text-[10px] font-medium px-1.5 py-0.5 rounded bg-edge text-muted"
              >{{
                r.key_type === "post_quantum"
                  ? t("setup.identity.badgePq")
                  : r.key_type === "plugin"
                    ? t("setup.identity.badgePlugin")
                    : t("setup.identity.badgeSsh")
              }}</span
            >
            <span
              v-if="matchedRecipient?.public_key === r.public_key"
              class="shrink-0 text-[10px] font-medium px-1.5 py-0.5 rounded bg-accent text-on-accent"
              >{{ t("setup.identity.badgeYourKey") }}</span
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
      <BaseIcon :icon="KeyRound" /> {{ t("setup.identity.reuseSshKey") }}
    </BaseButton>

    <div class="flex flex-col gap-1">
      <label for="identity" class="text-sm font-medium">{{
        t("setup.identity.identityLabel")
      }}</label>
      <BaseTextarea
        id="identity"
        v-model="identity"
        rows="5"
        class="masked-secret"
        :placeholder="t('setup.identity.identityPlaceholder')"
        autocomplete="off"
        spellcheck="false"
        :disabled="loadingIdentity || identitySource === 'file'"
      />

      <!-- Picked-file panel: the bytes live in backend state, not here -->
      <div
        v-if="identitySource === 'file' && pickedFile"
        class="flex flex-col gap-2 text-xs bg-input border border-edge rounded-md p-2 px-2.5"
      >
        <div class="flex items-center justify-between gap-2">
          <span class="min-w-0 truncate">
            <BaseIcon
              :icon="FileText"
              :size="14"
              class="inline-block align-middle shrink-0"
            />
            {{ pickedFile.filename || t("setup.identity.fileTypeFallback") }} ·
            {{ pickedFile.key_type
            }}<span v-if="pickedFile.encrypted">{{
              t("setup.identity.fileEncrypted")
            }}</span>
          </span>
          <button
            type="button"
            class="shrink-0 text-muted hover:text-danger active:text-danger transition-colors"
            @click="onUsePaste"
          >
            {{ t("setup.identity.fileRemove") }}
          </button>
        </div>

        <!-- Public key, once usable (unencrypted, or unlocked) -->
        <div v-if="pickedFile.recipient" class="flex flex-col gap-0.5">
          <span class="text-muted">{{
            t("setup.identity.filePublicKey")
          }}</span>
          <code class="font-mono break-all">{{
            truncateKey(pickedFile.recipient)
          }}</code>
        </div>

        <!-- Encrypted: unlock + verify before the key is usable -->
        <div v-else class="flex flex-col gap-1">
          <BaseInput
            v-model="passphrase"
            type="password"
            :placeholder="t('setup.identity.fileUnlockPlaceholder')"
            autocomplete="off"
            :disabled="verifying"
          />
          <BaseButton
            variant="secondary"
            :disabled="verifying || !passphrase"
            @click="onVerify"
          >
            {{
              verifying
                ? t("setup.identity.fileUnlockLoading")
                : t("setup.identity.fileUnlockButton")
            }}
          </BaseButton>
          <small class="text-muted">{{
            t("setup.identity.fileUnlockHint")
          }}</small>
        </div>
      </div>
      <small v-else class="text-xs text-muted">{{
        t("setup.identity.identityHint")
      }}</small>

      <!-- Upload via the native picker (hidden once a file is picked) -->
      <BaseButton
        v-if="identitySource !== 'file'"
        variant="secondary"
        size="sm"
        :disabled="picking || loadingIdentity"
        @click="onPickFile"
      >
        {{
          picking
            ? t("setup.identity.uploadButtonLoading")
            : t("setup.identity.uploadButton")
        }}
      </BaseButton>
    </div>

    <!-- Inline unsupported / malformed alerts for the pasted key. -->
    <BaseAlert v-if="identityType === 'post_quantum'" variant="warning">
      <BaseIcon
        :icon="TriangleAlert"
        :size="14"
        class="inline-block align-middle"
      />
      {{ t("setup.identity.unsupportedPq") }}
    </BaseAlert>
    <BaseAlert v-else-if="identityType === 'plugin'" variant="warning">
      <BaseIcon
        :icon="TriangleAlert"
        :size="14"
        class="inline-block align-middle"
      />
      {{ t("setup.identity.unsupportedPlugin") }}
    </BaseAlert>
    <BaseAlert v-else-if="malformedIdentity" variant="danger">
      <BaseIcon
        :icon="TriangleAlert"
        :size="14"
        class="inline-block align-middle"
      />
      {{ t("setup.identity.malformed") }}
    </BaseAlert>

    <!-- SSH key passphrase + Verify (paste path: required for an encrypted SSH key) -->
    <div
      v-if="identitySource === 'paste' && isSshIdentity && isIdentityEncrypted"
      class="flex flex-col gap-1"
    >
      <label for="passphrase" class="text-sm font-medium">{{
        t("setup.identity.sshPassphraseLabel")
      }}</label>
      <BaseInput
        id="passphrase"
        v-model="passphrase"
        type="password"
        :placeholder="t('setup.identity.sshPassphrasePlaceholder')"
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
        {{
          verifying
            ? t("setup.identity.sshVerifyButtonLoading")
            : t("setup.identity.sshVerifyButton")
        }}
      </BaseButton>
      <small class="text-xs text-muted">{{
        t("setup.identity.sshVerifyHint")
      }}</small>
    </div>

    <!-- Optional seal encryption (paste path; x25519 keys only) -->
    <PassphraseField
      v-else-if="identitySource === 'paste' && identityType === 'x25519'"
      ref="pf"
      id="identity-passphrase"
      v-model="passphrase"
      :label="t('setup.identity.sealPassphraseLabel')"
      :placeholder="t('setup.identity.sealPassphrasePlaceholder')"
      :optional="true"
      :disabled="loadingIdentity"
    >
      <template #help>
        <small class="text-xs text-muted">{{
          t("setup.identity.sealPassphraseHelpAge")
        }}</small>
        <BaseAlert v-if="!passphrase.trim()" variant="warning">
          <BaseIcon
            :icon="TriangleAlert"
            :size="14"
            class="inline-block align-middle"
          />
          {{ t("setup.identity.sealPassphraseWarning") }}
        </BaseAlert>
      </template>
    </PassphraseField>

    <!-- x25519 seal: forced unrecoverable ack (only once a passphrase is
         typed; empty = plaintext = no lockout risk). The SSH-key passphrase
         field above decrypts an existing key rather than setting a new seal
         passphrase, so it gets no ack. -->
    <PassphraseUnrecoverableAck
      v-if="
        identitySource === 'paste' && identityType === 'x25519' && passphrase
      "
      v-model="ackX25519"
    />

    <BaseAlert variant="info" class="text-center">
      {{ t("common.setup.storedLocally") }}
    </BaseAlert>

    <BaseAlert v-if="error" variant="danger">{{ error }}</BaseAlert>

    <BaseButton
      variant="primary"
      type="submit"
      :loading="loadingIdentity"
      :disabled="ackRequired"
      >{{
        loadingIdentity
          ? t("setup.identity.verifying")
          : t("setup.identity.buttonComplete")
      }}</BaseButton
    >
  </form>
</template>

<style scoped>
/* The pasted identity (an AGE-SECRET-KEY-1… line or a multi-line OPENSSH PEM) is
 * the app's most sensitive frontend string — render it masked, not in cleartext.
 * Purely presentational: -webkit-text-security glyphs every character while the
 * textarea stays multi-line (PEM-friendly), and the value/ref logic is
 * untouched. Scoped + class-fallthrough applies to BaseTextarea's <textarea>. */
.masked-secret {
  -webkit-text-security: disc;
}
</style>
