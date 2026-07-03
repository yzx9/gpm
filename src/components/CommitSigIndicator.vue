<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import type { CommitSigStatus } from "@/api";
import BaseIcon from "@/components/base/BaseIcon.vue";
import { signerFp, statusLabel } from "@/utils/signature";
import type { LucideIcon } from "@lucide/vue";
import {
  CircleAlert,
  CircleCheck,
  CircleDashed,
  CircleQuestionMark,
  CircleX,
} from "@lucide/vue";

type Tone = "success" | "warning" | "danger";

const props = withDefaults(
  defineProps<{
    status: CommitSigStatus;
    /** glyph: a coloured status icon (default). banner: the full detail box
     * with label + signer fingerprint. Layout classes (e.g. `mt-3`) can be
     * added by the caller via attribute fallthrough. */
    variant?: "glyph" | "banner";
    /** banner only: show an "ignored" chip at the trailing edge. */
    ignored?: boolean;
  }>(),
  { variant: "glyph", ignored: false },
);

/** kind → icon. Circle set keeps the row of status indicators visually
 * consistent. Every kind is listed explicitly so a future `CommitSigStatus`
 * variant forces a compile error here, matching the exhaustive `tone()` switch
 * and `statusLabel`. */
const ICON: Record<CommitSigStatus["kind"], LucideIcon> = {
  verified: CircleCheck,
  untrusted_key: CircleAlert,
  unsigned: CircleDashed,
  bad_signature: CircleX,
  unsupported_format: CircleQuestionMark,
  unknown: CircleQuestionMark,
};

/** Map a status to the semantic tone that colours both the icon and the banner.
 * This is the single source of truth that used to be spread across the removed
 * `statusClass`/`statusBgClass` helpers (verified→green, bad_signature→red,
 * everything else→amber). Every kind is listed explicitly (no `default`) so a
 * future `CommitSigStatus` variant forces a compile error here, matching the
 * exhaustive `ICON` map and `statusLabel` switch. */
function tone(status: CommitSigStatus): Tone {
  switch (status.kind) {
    case "verified":
      return "success";
    case "bad_signature":
      return "danger";
    case "unsigned":
    case "untrusted_key":
    case "unsupported_format":
    case "unknown":
      return "warning";
  }
}

// Literal class strings so Tailwind's scanner generates the utilities.
const GLYPH_TONE: Record<Tone, string> = {
  success: "text-success",
  warning: "text-warning",
  danger: "text-danger",
};
const BANNER_TONE: Record<Tone, string> = {
  success: "bg-success-soft text-success",
  warning: "bg-warning-soft text-warning",
  danger: "bg-danger-soft text-danger",
};
</script>

<template>
  <BaseIcon
    v-if="props.variant === 'glyph'"
    :icon="ICON[props.status.kind]"
    :class="GLYPH_TONE[tone(props.status)]"
  />
  <div
    v-else
    class="p-2 rounded-sm text-sm flex items-center gap-2"
    :class="BANNER_TONE[tone(props.status)]"
  >
    <BaseIcon :icon="ICON[props.status.kind]" :size="18" />
    <div class="flex-1 min-w-0">
      <div class="font-medium">{{ statusLabel(props.status) }}</div>
      <div v-if="signerFp(props.status)" class="text-xs text-muted break-all">
        {{ signerFp(props.status) }}
      </div>
    </div>
    <span
      v-if="props.ignored"
      class="text-[0.6rem] text-subtle px-1 rounded-sm bg-edge shrink-0"
      >ignored</span
    >
  </div>
</template>
