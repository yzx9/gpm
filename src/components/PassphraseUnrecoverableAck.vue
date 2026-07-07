<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import { TriangleAlert } from "@lucide/vue";
import { useI18n } from "vue-i18n";

// Forced acknowledgment that an identity passphrase cannot be recovered.
// Parent controls visibility (v-if — only show where a NEW passphrase is being
// established AND the user has typed one; empty optional = plaintext = no
// risk). This component owns the single source of the warning text + the
// checkbox visual; the "typed but not acknowledged" submit guard lives in each
// consumer's submit handler so a disabled button can't be bypassed via Enter.
const { t } = useI18n();

const acked = defineModel<boolean>({ default: false });
</script>

<template>
  <BaseAlert variant="warning" class="mt-1 flex gap-2">
    <BaseIcon :icon="TriangleAlert" :size="16" class="shrink-0 mt-0.5" />
    <div class="flex flex-col gap-2 min-w-0">
      <span>{{ t("common.passphraseAck.warning") }}</span>
      <label class="flex items-start gap-2 text-xs">
        <input type="checkbox" v-model="acked" class="mt-0.5" />
        <span>{{ t("common.passphraseAck.ack") }}</span>
      </label>
    </div>
  </BaseAlert>
</template>
