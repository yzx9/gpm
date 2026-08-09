<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<script setup lang="ts">
import {
  copyGeneratedPassword,
  ensureClipboardNotifyPermission,
  generatePasswordBatch,
  type AppError,
  type GenerateMode,
} from "@/api";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseHeader from "@/components/base/BaseHeader.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import BaseSelect from "@/components/base/BaseSelect.vue";
import { useSecureClaim, useToast, useWipeOnLeave } from "@/composables";
import { clipboardNotifyText } from "@/i18n/native";
import { Copy, Dices } from "@lucide/vue";
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

const { t } = useI18n();
const { toast } = useToast();

// ── Generator options ─────────────────────────────────────────────────────
const mode = ref<GenerateMode>("random");

const modeOptions = computed<{ label: string; value: GenerateMode }[]>(() => [
  { label: t("generate.genRandom"), value: "random" },
  { label: t("generate.genMemorable"), value: "memorable" },
  { label: t("generate.genPassphrase"), value: "xkcd" },
]);

function onModeChange(next: GenerateMode) {
  mode.value = next;
}
const length = ref(24);
const count = ref(10);

// ── Results ────────────────────────────────────────────────────────────────
const generated = ref<string[]>([]);
const generating = ref(false);
const error = ref("");
// Bumped on every generate and on lock; an in-flight generate whose token no
// longer matches is stale and must not write its result into the list.
let generateToken = 0;
// R031: the generated list is secret, so hold a screen-capture claim from the
// first result until unmount (onScopeDispose releases). Re-generate keeps it.
const { acquire: acquireSecure, release: releaseSecure } = useSecureClaim();
let secured = false;

// Length only applies to random (exact) and memorable (a minimum). xkcd is a
// fixed 4-word passphrase — word-count is a different unit, so hide the field.
const showLength = computed(() => mode.value !== "xkcd");
const lengthLabel = computed(() =>
  mode.value === "memorable" ? t("generate.lengthMin") : t("generate.length"),
);

// Number inputs can momentarily hold "" / NaN while editing; coerce before IPC
// so a transient empty field never sends a non-usize to the backend.
const safeCount = computed(() =>
  Number.isInteger(count.value) && count.value >= 1 ? count.value : 10,
);
const lenPayload = computed(() => {
  if (!showLength.value) return null;
  return Number.isInteger(length.value) && length.value >= 1
    ? length.value
    : null;
});

/** Generate a batch of passwords via the backend (CSPRNG). */
async function onGenerate() {
  const myToken = ++generateToken;
  generating.value = true;
  error.value = "";
  try {
    // min == max pins an exact length for random; memorable treats it as a
    // floor (word+digit repeated to ≥ min); null keeps the built-in default.
    const passwords = await generatePasswordBatch({
      mode: mode.value,
      charset: null,
      minLen: lenPayload.value,
      maxLen: lenPayload.value,
      strict: false,
      count: safeCount.value,
    });
    // A lock or a newer generate superseded this call — drop the result.
    if (myToken !== generateToken) return;
    // Raise FLAG_SECURE before the secrets paint (first result only; a failed
    // acquire aborts with a toast — the per-op replacement for the route abort).
    if (!secured) {
      const ok = await acquireSecure();
      if (myToken !== generateToken) {
        // A lock/newer generate superseded this call mid-acquire — drop the
        // claim we just took so it isn't stranded until unmount.
        releaseSecure();
        return;
      }
      if (!ok) {
        error.value = t("common.toast.secureScreenFailed");
        return;
      }
      secured = true;
    }
    generated.value = passwords;
  } catch (e) {
    if (myToken !== generateToken) return;
    const appError = e as AppError;
    error.value = appError?.message || t("generate.genFailed");
    console.error("[generate] generate failed", e);
  } finally {
    if (myToken === generateToken) generating.value = false;
  }
}

/** Copy one generated password; the backend arms the configured clipboard auto-clear. */
async function onCopyRow(pw: string) {
  try {
    await ensureClipboardNotifyPermission();
    await copyGeneratedPassword(pw, clipboardNotifyText());
    toast.success(t("common.toast.copied"));
  } catch (e) {
    const appError = e as AppError;
    toast.danger(appError?.message || t("common.toast.copyFailed"));
    console.warn("[generate] copy failed", e);
  }
}

// Wipe the batch on a hard identity lock, on browser back, or on unmount. The
// unlock modal can keep this page mounted behind the overlay on auto-lock, so
// unmount alone can't guarantee a wipe. Bumping generateToken also rejects any
// in-flight generate (a stale resolve can't repopulate the batch).
useWipeOnLeave(() => {
  generateToken++;
  generating.value = false;
  generated.value = [];
  // Drop the claim with the batch so a re-generate after a lock re-acquires
  // (and FLAG_SECURE doesn't stay up on an empty result list).
  if (secured) {
    releaseSecure();
    secured = false;
  }
});
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <BaseHeader :back-fallback="{ name: 'entries' }">
      <template #title>
        <h1 class="text-lg flex-1">{{ t("generate.title") }}</h1>
      </template>
    </BaseHeader>

    <BaseAlert v-if="error" variant="danger" class="mb-3">{{
      error
    }}</BaseAlert>

    <form class="controls" @submit.prevent="onGenerate">
      <BaseSelect
        name="generate-style"
        :legend="t('generate.style')"
        :model-value="mode"
        :options="modeOptions"
        :disabled="generating"
        @change="onModeChange"
      />

      <div v-if="showLength" class="flex flex-col gap-1">
        <label for="g-length" class="text-sm font-medium">
          {{ lengthLabel }}
        </label>
        <BaseInput
          id="g-length"
          v-model.number="length"
          type="number"
          min="1"
          max="256"
          :disabled="generating"
          :aria-label="t('generate.lengthAria')"
        />
      </div>

      <div class="flex flex-col gap-1">
        <label for="g-count" class="text-sm font-medium">{{
          t("generate.howMany")
        }}</label>
        <BaseInput
          id="g-count"
          v-model.number="count"
          type="number"
          min="1"
          max="32"
          :disabled="generating"
          :aria-label="t('generate.howManyAria')"
        />
      </div>

      <BaseButton variant="primary" type="submit" :disabled="generating">
        <BaseIcon v-if="!generating" :icon="Dices" />
        {{ generating ? t("generate.generating") : t("generate.generate") }}
      </BaseButton>
    </form>

    <ul v-if="generated.length" class="result-list" role="list">
      <li v-for="(pw, i) in generated" :key="i" class="result-row">
        <code class="result-pw">{{ pw }}</code>
        <BaseButton
          variant="secondary"
          size="sm"
          :aria-label="t('generate.copyAria')"
          @click="onCopyRow(pw)"
        >
          <BaseIcon :icon="Copy" />
        </BaseButton>
      </li>
    </ul>
  </main>
</template>

<style scoped>
.controls {
  display: flex;
  flex-direction: column;
  gap: 1rem;
  margin-bottom: 1.5rem;
}

.result-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.result-row {
  display: flex;
  align-items: stretch;
  gap: 0.5rem;
}

.result-pw {
  flex: 1 1 auto;
  display: flex;
  align-items: center;
  padding: 0.6rem 0.75rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  background: var(--color-surface);
  font-family: var(--font-mono, monospace);
  font-size: var(--text-sm);
  word-break: break-all;
  min-height: 48px;
}
</style>
