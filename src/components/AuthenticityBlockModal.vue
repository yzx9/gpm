<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<!-- Shared Enforce-block modal — reused by the Sync path and the save path's
     `authenticity_blocked` outcome. Lists the offending commits and offers the
     trust-signer / switch-to-Audit recovery actions. Lifted from EntryListPage
     so both entry points share one surface. -->

<script setup lang="ts">
import type { CommitSigInfo } from "@/api";
import { statusLabel } from "@/utils/signature";
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import BaseButton from "./base/BaseButton.vue";
import BaseModalShell from "./base/BaseModalShell.vue";
import CommitSigIndicator from "./CommitSigIndicator.vue";

const { t } = useI18n();

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
    aria-label="Sync blocked"
    @close="emit('close')"
  >
    <h2 class="text-base font-medium mb-1 text-danger">Sync blocked</h2>
    <p class="text-xs text-muted mb-3">
      Enforce mode refused to update the store — HEAD did not advance. Resolve
      the signature issue, then sync again.
    </p>
    <ul class="flex flex-col gap-2 mb-3">
      <li
        v-for="c in issues"
        :key="c.hash"
        class="flex items-center gap-2 text-sm"
      >
        <CommitSigIndicator :status="c.status" class="shrink-0" />
        <code class="text-xs text-muted">{{ c.short_hash }}</code>
        <span class="flex-1 truncate">{{ c.subject }}</span>
        <span class="text-xs text-muted">{{ statusLabel(c.status) }}</span>
      </li>
    </ul>
    <div class="flex flex-col gap-2">
      <p v-if="unverifiedIssue" class="text-xs text-muted mb-1 break-words">
        GPG-signed commit
        <code class="text-xs">{{ unverifiedIssue.short_hash }}</code> was made
        by a signer you haven't trusted. GPG signatures don't embed the public
        key, so open <strong>Settings → Trusted signing keys</strong> and add
        (or import) that signer's armored public key to verify it.
        <span v-if="unverifiedSignerFp">
          Issuer fingerprint:
          <code class="break-all">{{ unverifiedSignerFp }}</code>
        </span>
      </p>
      <BaseButton
        v-if="untrustedIssue"
        size="sm"
        @click="emit('trust-signer', untrustedIssue)"
      >
        Trust this signer
      </BaseButton>
      <BaseButton size="sm" @click="emit('switch-to-audit')">
        Switch to Audit mode
      </BaseButton>
      <BaseButton size="sm" @click="emit('close')">{{
        t("common.button.cancel")
      }}</BaseButton>
    </div>
  </BaseModalShell>
</template>
