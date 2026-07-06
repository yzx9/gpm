<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import {
  copyGeneratedPassword,
  generatePasswordBatch,
  type AppError,
  type GenerateMode,
} from "@/api";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import {
  ensureClipboardNotifyPermission,
  useLockState,
  useToast,
} from "@/composables";
import { navBack } from "@/utils/nav";
import { ArrowLeft, Copy, Dices } from "@lucide/vue";
import { computed, onBeforeUnmount, ref } from "vue";
import { useRouter } from "vue-router";

const router = useRouter();
const { onLock } = useLockState();
const { toast } = useToast();

// ── Generator options ─────────────────────────────────────────────────────
const mode = ref<GenerateMode>("random");
const length = ref(24);
const count = ref(10);

// ── Results ────────────────────────────────────────────────────────────────
const generated = ref<string[]>([]);
const generating = ref(false);
const error = ref("");
// Bumped on every generate and on lock; an in-flight generate whose token no
// longer matches is stale and must not write its result into the list.
let generateToken = 0;

// Length only applies to random (exact) and memorable (a minimum). xkcd is a
// fixed 4-word passphrase — word-count is a different unit, so hide the field.
const showLength = computed(() => mode.value !== "xkcd");
const lengthLabel = computed(() =>
  mode.value === "memorable" ? "Length (minimum)" : "Length",
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

function goBack() {
  navBack(router, { name: "entries" });
}

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
    generated.value = passwords;
  } catch (e) {
    if (myToken !== generateToken) return;
    const appError = e as AppError;
    error.value = appError?.message || "Could not generate passwords";
  } finally {
    if (myToken === generateToken) generating.value = false;
  }
}

/** Copy one generated password; the backend arms the configured clipboard auto-clear. */
async function onCopyRow(pw: string) {
  try {
    await ensureClipboardNotifyPermission();
    await copyGeneratedPassword(pw);
    toast.success("Copied — clipboard clears automatically");
  } catch (e) {
    const appError = e as AppError;
    toast.danger(appError?.message || "Could not copy");
  }
}

// The unlock modal keeps pages mounted on auto-lock, so wipe the batch the
// moment the identity locks (and cancel any in-flight generate).
onLock(() => {
  generateToken++;
  generating.value = false;
  generated.value = [];
});

onBeforeUnmount(() => {
  // Invalidate any in-flight generate so a late resolve can't repopulate the
  // batch after unmount (mirrors onLock).
  generateToken++;
  generating.value = false;
  generated.value = [];
});
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <header class="flex items-center gap-3 mb-6" role="banner">
      <button
        @click="goBack"
        class="back-btn inline-flex items-center gap-1"
        aria-label="Back"
      >
        <BaseIcon :icon="ArrowLeft" /> Back
      </button>
      <h1 class="text-lg flex-1">Generate password</h1>
    </header>

    <BaseAlert v-if="error" variant="danger" class="mb-3">{{
      error
    }}</BaseAlert>

    <form class="controls" @submit.prevent="onGenerate">
      <div class="flex flex-col gap-1">
        <label for="g-mode" class="text-sm font-medium">Style</label>
        <select
          id="g-mode"
          v-model="mode"
          class="gen-select"
          :disabled="generating"
          aria-label="Password style"
        >
          <option value="random">Random</option>
          <option value="memorable">Memorable</option>
          <option value="xkcd">Passphrase</option>
        </select>
      </div>

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
          aria-label="Length"
        />
      </div>

      <div class="flex flex-col gap-1">
        <label for="g-count" class="text-sm font-medium">How many</label>
        <BaseInput
          id="g-count"
          v-model.number="count"
          type="number"
          min="1"
          max="32"
          :disabled="generating"
          aria-label="How many"
        />
      </div>

      <BaseButton variant="primary" type="submit" :disabled="generating">
        <BaseIcon v-if="!generating" :icon="Dices" />
        {{ generating ? "Generating…" : "Generate" }}
      </BaseButton>
    </form>

    <ul v-if="generated.length" class="result-list" role="list">
      <li v-for="(pw, i) in generated" :key="i" class="result-row">
        <code class="result-pw">{{ pw }}</code>
        <button
          type="button"
          class="icon-btn"
          aria-label="Copy"
          @click="onCopyRow(pw)"
        >
          <BaseIcon :icon="Copy" />
        </button>
      </li>
    </ul>
  </main>
</template>

<style scoped>
.back-btn {
  background: transparent;
  border: none;
  font-size: var(--text-base);
  cursor: pointer;
  color: var(--color-accent);
  padding: 0.25rem 0.5rem;
  min-width: 48px;
  min-height: 48px;
}

.controls {
  display: flex;
  flex-direction: column;
  gap: 1rem;
  margin-bottom: 1.5rem;
}

.gen-select {
  padding: 0 0.5rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  background: var(--color-surface);
  color: inherit;
  font-size: var(--text-sm);
  min-height: 48px;
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

.icon-btn {
  flex: 0 0 auto;
  width: 48px;
  min-height: 48px;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  background: var(--color-surface);
  cursor: pointer;
  font-size: 1.1rem;
  line-height: 1;
  padding: 0;
}
.icon-btn:active:not(:disabled) {
  background: var(--color-hover);
}
@media (hover: hover) {
  .icon-btn:hover:not(:disabled) {
    background: var(--color-hover);
  }
}
.icon-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}
</style>
