<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<script setup lang="ts">
import type { AttributeView } from "@/api";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import { currentLocale, loadBundle } from "@/i18n";
import { Eye, EyeOff } from "@lucide/vue";
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

// Reuse the `entry.*` bundle (loaded for the read view); load it explicitly so
// the attribute eye-toggle labels resolve on any page that mounts this without a
// prior entry view (the revisions page, the conflict modal).
void loadBundle(currentLocale(), "entry");

const { t } = useI18n();

const props = defineProps<{ attributes: AttributeView[] }>();

/** Structural keys with dedicated UI (TOTP, attachments) — hidden from the
 *  attribute list so they don't duplicate the 2FA/Export affordances. The editor
 *  still sees them (so `otpauth` stays editable); only display filters them. */
const STRUCTURAL_KEYS = new Set([
  "totp",
  "otpauth",
  "content-transfer-encoding",
  "content-disposition",
]);

/** A key whose value is secret-bearing — masked like a password (framed + eye). */
function isSensitiveKey(key: string): boolean {
  return /password|secret|pin|token|passphrase|api[_-]?key|access[_-]?key|bearer|credential/i.test(
    key,
  );
}

/** User-facing attributes: structural keys hidden client-side. */
const visible = computed(() =>
  props.attributes.filter((a) => !STRUCTURAL_KEYS.has(a.key.toLowerCase())),
);

const sensitive = computed(() =>
  visible.value
    .map((attr, index) => ({ attr, index }))
    .filter((x) => isSensitiveKey(x.attr.key)),
);

const plain = computed(() =>
  visible.value
    .map((attr, index) => ({ attr, index }))
    .filter((x) => !isSensitiveKey(x.attr.key)),
);

/** Per-attribute reveal state for sensitive values (masked by default). */
const revealedValue = ref<Record<number, boolean>>({});
function toggleValue(index: number) {
  revealedValue.value[index] = !revealedValue.value[index];
}
</script>

<template>
  <div v-if="visible.length > 0" class="flex flex-col gap-2">
    <!-- Sensitive attributes: framed, masked, per-row eye toggle. -->
    <div
      v-for="{ attr, index } in sensitive"
      :key="`s-${index}`"
      class="rounded-sm bg-accent-ring p-2"
    >
      <label
        class="block text-xs font-semibold uppercase tracking-wide text-muted mb-1"
        >{{ attr.key }}</label
      >
      <div class="flex items-center gap-2">
        <code class="font-mono break-all flex-1 select-all">{{
          revealedValue[index] ? attr.value : "••••••••••"
        }}</code>
        <BaseButton
          variant="link"
          tone="muted"
          :aria-label="
            revealedValue[index]
              ? t('entry.hideValueAria')
              : t('entry.showValueAria')
          "
          @click="toggleValue(index)"
        >
          <BaseIcon :icon="revealedValue[index] ? EyeOff : Eye" />
        </BaseButton>
      </div>
    </div>

    <!-- Plain metadata: compact labeled rows, source order. -->
    <div
      v-for="{ attr, index } in plain"
      :key="`p-${index}`"
      class="flex flex-col"
    >
      <span class="text-xs font-semibold uppercase tracking-wide text-muted">{{
        attr.key
      }}</span>
      <span class="break-all">{{ attr.value }}</span>
    </div>
  </div>
</template>
