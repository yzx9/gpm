<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<!-- Shared Enforce-block modal — reused by the Sync path and the save path's
     `authenticity_blocked` outcome. Lists the offending commits and offers the
     trust-signer / switch-to-Audit recovery actions. Lifted from EntryListPage
     so both entry points share one surface. -->

<script setup lang="ts">
import type { CommitSigInfo } from "@/api";
import { statusGlyph, statusLabel } from "@/utils/signature";
import { computed } from "vue";
import BaseButton from "./base/BaseButton.vue";
import BaseModalShell from "./base/BaseModalShell.vue";

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
        <span class="text-lg" aria-hidden="true">{{
          statusGlyph(c.status)
        }}</span>
        <code class="text-xs text-muted">{{ c.short_hash }}</code>
        <span class="flex-1 truncate">{{ c.subject }}</span>
        <span class="text-xs text-muted">{{ statusLabel(c.status) }}</span>
      </li>
    </ul>
    <div class="flex flex-col gap-2">
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
      <BaseButton size="sm" @click="emit('close')">Cancel</BaseButton>
    </div>
  </BaseModalShell>
</template>
