<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<script setup lang="ts">
import {
  copyGeneratedPassword,
  DEFAULT_GENERATOR_SETTINGS,
  ensureClipboardNotifyPermission,
  generatePasswordBatch,
  getAppConfig,
  normalizeGeneratorSettings,
  setGeneratorSettings,
  type AppError,
  type GenerateMode,
  type GeneratorSettings,
} from "@/api";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseHeader from "@/components/base/BaseHeader.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import BaseOnOffToggle from "@/components/base/BaseOnOffToggle.vue";
import BaseSegmentedControl from "@/components/base/BaseSegmentedControl.vue";
import { useDialog, useSecureClaim, useToast, useWipeOnLeave } from "@/composables";
import { clipboardNotifyText } from "@/i18n/native";
import { Copy, RefreshCw, Trash2 } from "@lucide/vue";
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

const { t } = useI18n();
const { toast } = useToast();
const { dialog } = useDialog();

// ── Generator options (persisted, non-secret) ──────────────────────────────
const mode = ref<GenerateMode>(DEFAULT_GENERATOR_SETTINGS.mode);
const length = ref(DEFAULT_GENERATOR_SETTINGS.length);
const count = ref(DEFAULT_GENERATOR_SETTINGS.count);

interface CharGroups {
  upper: boolean;
  lower: boolean;
  digits: boolean;
  symbols: boolean;
}

const groups = ref<CharGroups>({
  upper: DEFAULT_GENERATOR_SETTINGS.upper,
  lower: DEFAULT_GENERATOR_SETTINGS.lower,
  digits: DEFAULT_GENERATOR_SETTINGS.digits,
  symbols: DEFAULT_GENERATOR_SETTINGS.symbols,
});
const minDigits = ref(DEFAULT_GENERATOR_SETTINGS.minDigits);
const minSymbols = ref(DEFAULT_GENERATOR_SETTINGS.minSymbols);
const avoidAmbiguous = ref(DEFAULT_GENERATOR_SETTINGS.avoidAmbiguous);

const modeOptions = computed<{ label: string; value: GenerateMode }[]>(() => [
  { label: t("generate.genRandom"), value: "random" },
  { label: t("generate.genMemorable"), value: "memorable" },
  { label: t("generate.genPassphrase"), value: "xkcd" },
]);

// The exact alphabets the user can include. Symbols match gopass's
// `--symbols` intent but use a curated, form-safe set (see the backend's
// default alphabet for the broader internal set).
const GROUP_DEFS: { key: keyof CharGroups; label: string; chars: string }[] = [
  { key: "upper", label: "A-Z", chars: "ABCDEFGHIJKLMNOPQRSTUVWXYZ" },
  { key: "lower", label: "a-z", chars: "abcdefghijklmnopqrstuvwxyz" },
  { key: "digits", label: "0-9", chars: "0123456789" },
  { key: "symbols", label: "!@#$%^&*-_=+?", chars: "!@#$%^&*-_=+?" },
];

function groupLabel(key: keyof CharGroups): string {
  const labels: Record<keyof CharGroups, string> = {
    upper: t("generate.groupUpper"),
    lower: t("generate.groupLower"),
    digits: t("generate.groupDigits"),
    symbols: t("generate.groupSymbols"),
  };
  return labels[key];
}

const selectedCharset = computed(() =>
  GROUP_DEFS.filter((g) => groups.value[g.key])
    .map((g) => g.chars)
    .join(""),
);

const isRandom = computed(() => mode.value === "random");
// Length only applies to random (exact) and memorable (a minimum). xkcd is a
// fixed 4-word passphrase — word-count is a different unit, so hide the field.
const showLength = computed(() => mode.value !== "xkcd");
const lengthLabel = computed(() =>
  mode.value === "memorable" ? t("generate.lengthMin") : t("generate.length"),
);

// Number inputs can momentarily hold "" / NaN while editing; coerce before IPC
// so a transient empty field never sends a non-usize to the backend.
const safeCount = computed(() =>
  Number.isInteger(count.value)
    ? Math.min(32, Math.max(1, count.value))
    : DEFAULT_GENERATOR_SETTINGS.count,
);
const lenPayload = computed(() => {
  if (!showLength.value) return null;
  return Number.isInteger(length.value) ? length.value : null;
});

const validationError = computed(() => {
  if (!showLength.value) return "";
  if (!Number.isInteger(length.value) || length.value < 5 || length.value > 128) {
    return t("generate.errLength");
  }
  if (!isRandom.value) return "";
  if (selectedCharset.value.length === 0) return t("generate.errNoGroups");
  if (
    !Number.isInteger(minDigits.value) ||
    minDigits.value < 0 ||
    !Number.isInteger(minSymbols.value) ||
    minSymbols.value < 0
  ) {
    return t("generate.errMinValues");
  }
  if (minDigits.value > 0 && !groups.value.digits) return t("generate.errDigitGroup");
  if (minSymbols.value > 0 && !groups.value.symbols) return t("generate.errSymbolGroup");
  if (minDigits.value + minSymbols.value > length.value) {
    return t("generate.errMinSum");
  }
  return "";
});

// ── Settings persistence ───────────────────────────────────────────────────
function applySettings(settings: GeneratorSettings) {
  mode.value = settings.mode;
  length.value = settings.length;
  count.value = settings.count;
  groups.value = {
    upper: settings.upper,
    lower: settings.lower,
    digits: settings.digits,
    symbols: settings.symbols,
  };
  minDigits.value = settings.minDigits;
  minSymbols.value = settings.minSymbols;
  avoidAmbiguous.value = settings.avoidAmbiguous;
}

function snapshotSettings(): GeneratorSettings {
  return normalizeGeneratorSettings({
    mode: mode.value,
    length: length.value,
    count: count.value,
    ...groups.value,
    minDigits: minDigits.value,
    minSymbols: minSymbols.value,
    avoidAmbiguous: avoidAmbiguous.value,
  });
}

// Serialize saves so rapid slider/group edits persist in the order the user
// made them; the backend's write mutex can then never see an older snapshot
// arrive last. Failures are reported once per streak (a persistent backend
// problem shouldn't spam a toast on every slider tick).
let saveChain: Promise<void> = Promise.resolve();
let saveFailed = false;

function persistSettings() {
  const snapshot = snapshotSettings();
  saveChain = saveChain.then(async () => {
    try {
      await setGeneratorSettings(snapshot);
      saveFailed = false;
    } catch (e) {
      console.warn("[generate] save settings failed", e);
      if (!saveFailed) {
        saveFailed = true;
        toast.danger(t("generate.settingsSaveFailed"));
      }
    }
  });
}

// ── Results / history ──────────────────────────────────────────────────────
interface HistoryEntry {
  id: number;
  value: string;
}

const HISTORY_LIMIT = 50;
/** The most recent batch — the prominent result block above the controls. */
const current = ref<HistoryEntry[]>([]);
/** Earlier batches, newest first. Passwords only: never written to disk. */
const history = ref<HistoryEntry[]>([]);
let nextHistoryId = 1;
const generating = ref(false);
const error = ref("");
// Bumped on every generate and on lock; an in-flight generate whose token no
// longer matches is stale and must not write its result into the lists.
let generateToken = 0;
let disposed = false;
// R031: the generated list is secret, so hold a screen-capture claim from the
// first result until unmount (onScopeDispose releases). Re-generate keeps it.
const { acquire: acquireSecure, release: releaseSecure } = useSecureClaim();
let secured = false;

/** Drop both the current batch and the earlier history, releasing the
 *  screen-capture claim. Shared by the lock, leave, and unmount paths so a
 *  claim can never outlive the secrets. */
function clearGenerated(): boolean {
  const hadSecrets = current.value.length > 0 || history.value.length > 0;
  current.value = [];
  history.value = [];
  if (secured) {
    releaseSecure();
    secured = false;
  }
  return hadSecrets;
}

/** Generate a batch via the backend (CSPRNG). The previous batch moves into
 *  history, so "regenerate" never discards the value the user just saw. */
async function onGenerate() {
  if (validationError.value) {
    error.value = validationError.value;
    return;
  }
  const myToken = ++generateToken;
  generating.value = true;
  error.value = "";
  try {
    // min == max pins an exact length for random; memorable treats it as a
    // floor (word+digit repeated to ≥ min); null keeps the built-in default.
    const passwords = await generatePasswordBatch({
      mode: mode.value,
      charset: isRandom.value ? selectedCharset.value : null,
      minLen: lenPayload.value,
      maxLen: lenPayload.value,
      strict: false,
      minDigits: isRandom.value ? minDigits.value : 0,
      minSymbols: isRandom.value ? minSymbols.value : 0,
      avoidAmbiguous: isRandom.value ? avoidAmbiguous.value : false,
      count: safeCount.value,
    });
    // A lock, a newer generate, or an unmount superseded this call — drop it.
    if (myToken !== generateToken || disposed) return;
    // Raise FLAG_SECURE before the secrets paint (first result only; a failed
    // acquire aborts with a toast — the per-op replacement for the route abort).
    if (!secured) {
      const ok = await acquireSecure();
      if (myToken !== generateToken || disposed) {
        // A lock/newer generate/unmount superseded this call mid-acquire —
        // drop the claim we just took so it isn't stranded.
        releaseSecure();
        return;
      }
      if (!ok) {
        error.value = t("common.toast.secureScreenFailed");
        return;
      }
      secured = true;
    }
    const batch = passwords.map((value) => ({ id: nextHistoryId++, value }));
    if (current.value.length > 0) {
      history.value = [...current.value, ...history.value].slice(0, HISTORY_LIMIT);
    }
    current.value = batch;
  } catch (e) {
    if (myToken !== generateToken || disposed) return;
    const appError = e as AppError;
    error.value = appError?.message || t("generate.genFailed");
    console.error("[generate] generate failed", e);
  } finally {
    if (myToken === generateToken) generating.value = false;
  }
}

/** A committed option change (slider release, group toggle, blur) persists the
 *  recipe and refreshes the result, mirroring Bitwarden's live regeneration. */
function onSettingsCommitted() {
  persistSettings();
  void onGenerate();
}

function onModeChange(next: GenerateMode) {
  if (mode.value === next) return;
  mode.value = next;
  onSettingsCommitted();
}

function onGroupToggle(key: keyof CharGroups, enabled: boolean) {
  // Keep at least one group selected — an alphabet-less request can only fail.
  if (!enabled && !GROUP_DEFS.some((g) => g.key !== key && groups.value[g.key])) {
    // The DOM checkbox already flipped; re-render so it snaps back on.
    groups.value = { ...groups.value };
    return;
  }
  groups.value[key] = enabled;
  // A quota for a disabled class would make every request fail; clearing it
  // keeps the form in a generate-able state when a group is switched off.
  if (key === "digits" && !enabled) minDigits.value = 0;
  if (key === "symbols" && !enabled) minSymbols.value = 0;
  onSettingsCommitted();
}

function onAvoidAmbiguousChange(enabled: boolean) {
  avoidAmbiguous.value = enabled;
  onSettingsCommitted();
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

/** Clear only the earlier batches; the current result stays on screen. */
async function onClearHistory() {
  if (generating.value || history.value.length === 0) return;
  const confirmed = await dialog.confirm({
    message: t("generate.clearHistoryConfirm"),
    confirmLabel: t("common.button.clear"),
    danger: true,
  });
  if (!confirmed) return;
  history.value = [];
  if (!current.value.length && secured) {
    releaseSecure();
    secured = false;
  }
  toast.success(t("generate.historyCleared"));
}

onMounted(async () => {
  try {
    const cfg = await getAppConfig();
    applySettings(normalizeGeneratorSettings(cfg?.generator));
  } catch (e) {
    // A config read failure must not block the generator — fall back to the
    // install defaults, then generate immediately.
    console.warn("[generate] could not load settings", e);
    applySettings(normalizeGeneratorSettings(undefined));
  }
  if (disposed) return;
  await onGenerate();
});

onBeforeUnmount(() => {
  disposed = true;
});

// Wipe both lists on a hard identity lock, on browser back, or on unmount. The
// unlock modal can keep this page mounted behind the overlay on auto-lock, so
// unmount alone can't guarantee a wipe. Bumping generateToken also rejects any
// in-flight generate (a stale resolve can't repopulate the lists).
useWipeOnLeave(() => {
  generateToken++;
  generating.value = false;
  // An unsaved generated batch is user content lost to the lock — report it
  // so the post-unlock toast fires.
  return clearGenerated();
});
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <BaseHeader :back-fallback="{ name: 'entries' }">
      <template #title>
        <h1 class="text-lg flex-1">{{ t("generate.title") }}</h1>
      </template>
    </BaseHeader>

    <BaseAlert v-if="error" variant="danger" class="mb-3">{{ error }}</BaseAlert>

    <section class="generator">
      <BaseSegmentedControl
        name="generate-mode"
        :legend="t('generate.style')"
        :model-value="mode"
        :options="modeOptions"
        :disabled="generating"
        @change="onModeChange"
      />

      <div class="result-card" aria-live="polite">
        <ul v-if="current.length" class="result-list" role="list">
          <li v-for="entry in current" :key="entry.id" class="result-row current-row">
            <code class="result-pw current-pw">{{ entry.value }}</code>
            <BaseButton
              variant="secondary"
              size="sm"
              :aria-label="t('generate.copyAria')"
              @click="onCopyRow(entry.value)"
            >
              <BaseIcon :icon="Copy" />
            </BaseButton>
          </li>
        </ul>
        <p v-else-if="generating" class="text-sm text-muted">{{ t("generate.generating") }}</p>
        <p v-else class="text-sm text-muted">{{ t("generate.emptyResult") }}</p>
      </div>

      <BaseButton
        variant="primary"
        block
        :disabled="generating || !!validationError"
        :loading="generating"
        :aria-label="t('generate.regenerate')"
        @click="onGenerate"
      >
        <BaseIcon v-if="!generating" :icon="RefreshCw" />
        {{ generating ? t("generate.generating") : t("generate.regenerate") }}
      </BaseButton>

      <div v-if="showLength" class="slider-field">
        <div class="slider-head">
          <label for="g-length" class="text-sm font-medium">
            {{ lengthLabel }}
          </label>
          <output class="slider-value" for="g-length">{{ length }}</output>
        </div>
        <input
          id="g-length"
          v-model.number="length"
          class="slider"
          type="range"
          min="5"
          max="128"
          :disabled="generating"
          :aria-label="t('generate.lengthAria')"
          @change="onSettingsCommitted"
        />
      </div>

      <div class="slider-field">
        <div class="slider-head">
          <label for="g-count" class="text-sm font-medium">{{ t("generate.howMany") }}</label>
          <output class="slider-value" for="g-count">{{ count }}</output>
        </div>
        <input
          id="g-count"
          v-model.number="count"
          class="slider"
          type="range"
          min="1"
          max="32"
          :disabled="generating"
          :aria-label="t('generate.howManyAria')"
          @change="onSettingsCommitted"
        />
      </div>

      <template v-if="isRandom">
        <fieldset class="group-fieldset" :disabled="generating">
          <legend class="text-sm font-medium mb-1">{{ t("generate.groups") }}</legend>
          <div class="group-grid">
            <label
              v-for="g in GROUP_DEFS"
              :key="g.key"
              class="group-option"
              :class="{ active: groups[g.key] }"
            >
              <input
                type="checkbox"
                :checked="groups[g.key]"
                @change="onGroupToggle(g.key, ($event.target as HTMLInputElement).checked)"
              />
              <span class="group-copy">
                <code class="group-chars">{{ g.label }}</code>
                <span class="text-xs text-muted">{{ groupLabel(g.key) }}</span>
              </span>
            </label>
          </div>
        </fieldset>

        <BaseOnOffToggle
          name="generate-avoid-ambiguous"
          :legend="t('generate.avoidAmbiguous')"
          :model-value="avoidAmbiguous"
          :disabled="generating"
          @change="onAvoidAmbiguousChange"
        >
          <template #hint>
            <p class="text-xs text-muted">{{ t("generate.avoidAmbiguousHint") }}</p>
          </template>
        </BaseOnOffToggle>

        <div class="min-grid">
          <div class="flex flex-col gap-1">
            <label for="g-min-digits" class="text-sm font-medium">
              {{ t("generate.minDigits") }}
            </label>
            <BaseInput
              id="g-min-digits"
              v-model.number="minDigits"
              type="number"
              min="0"
              :max="length"
              :disabled="generating || !groups.digits"
              :aria-label="t('generate.minDigitsAria')"
              @change="onSettingsCommitted"
            />
          </div>
          <div class="flex flex-col gap-1">
            <label for="g-min-symbols" class="text-sm font-medium">
              {{ t("generate.minSymbols") }}
            </label>
            <BaseInput
              id="g-min-symbols"
              v-model.number="minSymbols"
              type="number"
              min="0"
              :max="length"
              :disabled="generating || !groups.symbols"
              :aria-label="t('generate.minSymbolsAria')"
              @change="onSettingsCommitted"
            />
          </div>
        </div>
      </template>
    </section>

    <section v-if="history.length" class="history" aria-labelledby="g-history-heading">
      <div class="history-head">
        <h2 id="g-history-heading" class="text-base font-medium">{{ t("generate.history") }}</h2>
        <BaseButton
          variant="link"
          tone="danger"
          size="sm"
          :disabled="generating"
          :aria-label="t('generate.clearHistory')"
          @click="onClearHistory"
        >
          <BaseIcon :icon="Trash2" :size="16" />
          {{ t("generate.clearHistory") }}
        </BaseButton>
      </div>
      <p class="text-xs text-muted mb-2">{{ t("generate.historyHint") }}</p>
      <ul class="result-list" role="list">
        <li v-for="entry in history" :key="entry.id" class="result-row history-row">
          <code class="result-pw">{{ entry.value }}</code>
          <BaseButton
            variant="secondary"
            size="sm"
            :aria-label="t('generate.copyAria')"
            @click="onCopyRow(entry.value)"
          >
            <BaseIcon :icon="Copy" />
          </BaseButton>
        </li>
      </ul>
    </section>
    <p v-else-if="!current.length && !generating" class="text-xs text-muted">
      {{ t("generate.emptyHistory") }}
    </p>
  </main>
</template>

<style scoped>
.generator {
  display: flex;
  flex-direction: column;
  gap: 1rem;
  margin-bottom: 1.5rem;
}

/* The prominent generated value(s), on a recessed well so the output reads as
   the focal point above the recipe. */
.result-card {
  padding: 0.6rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  background: var(--color-input);
}

.current-pw {
  font-size: var(--text-md);
}

.slider-field {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.slider-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 0.75rem;
}

.slider-value {
  font-size: var(--text-sm);
  font-variant-numeric: tabular-nums;
  color: var(--color-muted);
}

.slider {
  width: 100%;
  height: 2.5rem;
  margin: 0;
  background: transparent;
  cursor: pointer;
  -webkit-appearance: none;
  appearance: none;
}

.slider::-webkit-slider-runnable-track {
  height: 4px;
  border-radius: 999px;
  background: var(--color-edge);
}

.slider::-webkit-slider-thumb {
  width: 22px;
  height: 22px;
  margin-top: -9px;
  border: 2px solid var(--color-surface);
  border-radius: 50%;
  background: var(--color-accent);
  box-shadow: 0 1px 4px rgb(0 0 0 / 0.25);
  -webkit-appearance: none;
  appearance: none;
}

.slider::-moz-range-track {
  height: 4px;
  border-radius: 999px;
  background: var(--color-edge);
}

.slider::-moz-range-thumb {
  width: 22px;
  height: 22px;
  border: 2px solid var(--color-surface);
  border-radius: 50%;
  background: var(--color-accent);
  box-shadow: 0 1px 4px rgb(0 0 0 / 0.25);
}

.slider:focus-visible {
  outline: 2px solid var(--color-accent);
  outline-offset: 2px;
  border-radius: var(--radius-sm);
}

.group-fieldset {
  border: 0;
  padding: 0;
  margin: 0;
  min-width: 0;
}

.group-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 0.5rem;
}

.group-option {
  display: flex;
  align-items: center;
  gap: 0.6rem;
  padding: 0.55rem 0.7rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  background: var(--color-input);
  min-height: 48px;
  cursor: pointer;
  -webkit-tap-highlight-color: transparent;
}

.group-option.active {
  border-color: var(--color-accent);
  box-shadow: 0 0 0 2px var(--color-accent-ring);
}

.group-fieldset:disabled .group-option {
  opacity: 0.55;
  pointer-events: none;
  cursor: default;
}

.group-option input {
  width: 1.1rem;
  height: 1.1rem;
  accent-color: var(--color-accent);
  flex: 0 0 auto;
}

.group-copy {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.group-chars {
  font-family: var(--font-mono, monospace);
  font-size: var(--text-sm);
}

.min-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 0.75rem;
}

.history-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.5rem;
}

.history {
  margin-top: 0.5rem;
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
