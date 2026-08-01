<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<!-- Shared Enforce-block modal — reused by the Sync path and the save path's
     `authenticity_blocked` outcome. Lists the offending commits and offers the
     trust-signer / switch-to-Audit recovery actions. Lifted from EntryListPage
     so both entry points share one surface. -->

<script setup lang="ts">
import type { CommitSigInfo } from "@/api";
import { useCommitSignature } from "@/composables";
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import BaseButton from "./base/BaseButton.vue";
import BaseModalShell from "./base/BaseModalShell.vue";
import CommitSigIndicator from "./CommitSigIndicator.vue";

const { t } = useI18n();
const { signatureLabel } = useCommitSignature();

const props = defineProps<{
  /** Non-null shows the modal. */
  issues: CommitSigInfo[] | null;
}>();

const emit = defineEmits<{
  (e: "trust-signer", commit: CommitSigInfo): void;
  (e: "switch-to-audit"): void;
  (e: "close"): void;
}>();

/** The first untrusted-key issue, if any — gates the "Trust this signer" button. */
const untrustedIssue = computed(() =>
  props.issues?.find((c) => c.status.kind === "untrusted_key"),
);

/** The first GPG `unverified_signature` issue, if any. GPG signatures carry no
 * public key to auto-trust (unlike SSH-sig), so there's no "Trust this signer"
 * button for these — the only recovery is to add (or import) the signer's
 * armored public key in Settings. This gates that hint. */
const unverifiedIssue = computed(() =>
  props.issues?.find((c) => c.status.kind === "unverified_signature"),
);

/** The issuer fingerprint the unverified signature claimed (hex), so the user
 * can find the right key to paste. */
const unverifiedSignerFp = computed(() => {
  const issue = unverifiedIssue.value;
  return issue && issue.status.kind === "unverified_signature"
    ? issue.status.signer_fp
    : null;
});
</script>

<template>
  <BaseModalShell
    v-if="issues"
    variant="sheet"
    role="alertdialog"
    :aria-label="t('common.authenticity.ariaLabel')"
    @close="emit('close')"
  >
    <h2 class="text-base font-medium mb-1 text-danger">
      {{ t("common.authenticity.heading") }}
    </h2>
    <p class="text-xs text-muted mb-3">{{ t("common.authenticity.body") }}</p>
    <ul class="flex flex-col gap-2 mb-3">
      <li
        v-for="c in issues"
        :key="c.hash"
        class="flex items-center gap-2 text-sm"
      >
        <CommitSigIndicator :status="c.status" class="shrink-0" />
        <code class="text-xs text-muted">{{ c.short_hash }}</code>
        <span class="flex-1 truncate">{{ c.subject }}</span>
        <span class="text-xs text-muted">{{ signatureLabel(c.status) }}</span>
      </li>
    </ul>
    <div class="flex flex-col gap-2">
      <i18n-t
        v-if="unverifiedIssue"
        keypath="common.authenticity.gpgNotice"
        tag="p"
        class="text-xs text-muted mb-1 break-words"
      >
        <template #hash>
          <code class="text-xs">{{ unverifiedIssue.short_hash }}</code>
        </template>
        <template #link>
          <strong>{{ t("common.authenticity.gpgNoticeLink") }}</strong>
        </template>
      </i18n-t>
      <p
        v-if="unverifiedIssue && unverifiedSignerFp"
        class="text-xs text-muted mb-1 break-words"
      >
        {{ t("common.authenticity.unverifiedFingerprint") }}
        <code class="break-all">{{ unverifiedSignerFp }}</code>
      </p>
      <BaseButton
        v-if="untrustedIssue"
        size="sm"
        @click="emit('trust-signer', untrustedIssue)"
      >
        {{ t("common.authenticity.trustSigner") }}
      </BaseButton>
      <BaseButton size="sm" @click="emit('switch-to-audit')">
        {{ t("common.authenticity.switchToAudit") }}
      </BaseButton>
      <BaseButton size="sm" @click="emit('close')">{{
        t("common.button.cancel")
      }}</BaseButton>
    </div>
  </BaseModalShell>
</template>
