<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import { Eye, EyeOff } from "@lucide/vue";
import { computed, ref, useId, watch } from "vue";
import { useI18n } from "vue-i18n";

const { t } = useI18n();

const props = withDefaults(
  defineProps<{
    modelValue: string;
    /** Forwarded to the main input; the confirm input becomes `<id>-confirm`.
     *  Falls back to a generated id when omitted. Keeping it stable lets callers
     *  (and tests) address each field by id. */
    id?: string;
    /** Override the main field label; defaults to the localized "Passphrase".
     *  Plain-string override (no default) so `withDefaults` does not reference
     *  `t()` — `<script setup>` hoists `defineProps` out of `setup()`, so the
     *  default must not call into `useI18n()`. The localized fallback is
     *  resolved via {@link labelText}. */
    label?: string;
    placeholder?: string;
    confirmLabel?: string;
    confirmPlaceholder?: string;
    autocomplete?: string;
    disabled?: boolean;
    /** When true, an empty passphrase is valid (plaintext) and the confirm
     *  field only appears once the user actually types something. */
    optional?: boolean;
  }>(),
  {
    autocomplete: "new-password",
    disabled: false,
    optional: false,
  },
);

// Localized fallbacks for the optional label/placeholder props. Resolved here
// (not via `withDefaults`) because `<script setup>` hoists `defineProps` out of
// `setup()`, so the defaults cannot call `useI18n()`. Each falls back to the
// `common.passphraseField.*` value when the caller leaves the prop unset.
const labelText = computed(
  () => props.label ?? t("common.passphraseField.label"),
);
const placeholderText = computed(
  () => props.placeholder ?? t("common.passphraseField.placeholder"),
);
const confirmLabelText = computed(
  () => props.confirmLabel ?? t("common.passphraseField.confirmLabel"),
);
const confirmPlaceholderText = computed(
  () =>
    props.confirmPlaceholder ?? t("common.passphraseField.confirmPlaceholder"),
);

const emit = defineEmits<{ "update:modelValue": [string] }>();

// The confirm value lives here, not the parent — it is validation-only and
// never submitted (only `modelValue` crosses to the backend).
const confirm = ref("");
const show = ref(false);

const generatedMainId = useId();
const generatedConfirmId = useId();
const mainId = computed(() => props.id ?? generatedMainId);
const confirmId = computed(() =>
  props.id ? `${props.id}-confirm` : generatedConfirmId,
);

// Show the confirm field for required passphrases, or — for optional ones —
// only once the user has typed something (empty optional = plaintext; there is
// nothing to confirm until they choose a passphrase).
const showConfirmField = computed(
  () => !props.optional || props.modelValue !== "",
);

const mismatch = computed(() => props.modelValue !== confirm.value);
const inlineError = computed(
  () => confirm.value !== "" && props.modelValue !== "" && mismatch.value,
);

// A stale confirm on an emptied optional field could otherwise hide a mismatch
// (the field is hidden, so the user wouldn't see it). Clear it whenever the
// main field is emptied.
watch(
  () => props.modelValue,
  (v) => {
    if (v === "") confirm.value = "";
  },
);

function onMain(value: string | number | undefined) {
  emit("update:modelValue", value === undefined ? "" : String(value));
}

defineExpose({
  /** Returns an error message if invalid, null if valid — mirrors the per-form
   *  `validate(): string | null` convention so callers gate submit the same way. */
  validate(): string | null {
    if (props.modelValue === "") {
      return props.optional
        ? null
        : t("common.passphraseField.errRequired", { label: labelText.value });
    }
    if (confirm.value === "") return t("common.passphraseField.errConfirm");
    if (mismatch.value) return t("common.passphraseField.errMismatch");
    return null;
  },
  /** Clear the confirm field + re-hide — call on context switches (identity-kind
   *  change, regenerate) so a stale confirm can't persist. */
  reset() {
    confirm.value = "";
    show.value = false;
  },
});
</script>

<template>
  <div class="flex flex-col gap-1">
    <label :for="mainId" class="text-sm font-medium">{{ labelText }}</label>
    <div class="relative">
      <BaseInput
        :id="mainId"
        :model-value="modelValue"
        :type="show ? 'text' : 'password'"
        :placeholder="placeholderText"
        :autocomplete="autocomplete"
        :disabled="disabled"
        class="w-full"
        :style="{ paddingRight: '2.5rem' }"
        @update:model-value="onMain"
      />
      <button
        type="button"
        class="absolute inset-y-0 right-0 px-3 text-muted hover:text-accent active:text-accent transition-colors"
        :aria-label="
          show
            ? t('common.passphraseField.hide')
            : t('common.passphraseField.show')
        "
        @click="show = !show"
      >
        <BaseIcon :icon="show ? EyeOff : Eye" :size="18" />
      </button>
    </div>

    <template v-if="showConfirmField">
      <label :for="confirmId" class="text-sm font-medium mt-1">{{
        confirmLabelText
      }}</label>
      <div class="relative">
        <BaseInput
          :id="confirmId"
          :model-value="confirm"
          :type="show ? 'text' : 'password'"
          :placeholder="confirmPlaceholderText"
          :autocomplete="autocomplete"
          :disabled="disabled"
          class="w-full"
          :style="{ paddingRight: '2.5rem' }"
          @update:model-value="
            (v) => (confirm = v === undefined ? '' : String(v))
          "
        />
      </div>
      <small v-if="inlineError" class="text-xs text-danger">{{
        t("common.passphraseField.mismatch")
      }}</small>
    </template>

    <slot name="help" />
  </div>
</template>
