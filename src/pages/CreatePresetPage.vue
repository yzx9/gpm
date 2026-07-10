<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import {
  createFromPresetSecret,
  generatePassword,
  listCreatePresets,
  type AppError,
  type CreatePreset,
  type DivergenceChoice,
  type GenerateMode,
  type PresetField,
  type PullResult,
} from "@/api";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import BaseSpinner from "@/components/base/BaseSpinner.vue";
import {
  isAuthCancelled,
  useDivergence,
  useLockState,
  useToast,
  useWipeOnLeave,
} from "@/composables";
import { currentLocale, loadBundle } from "@/i18n";
import { navBack } from "@/utils/nav";
import { ArrowLeft, Dices, Eye, EyeOff } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";

// Reuse the `create.*` bundle (loaded for the pick step); load explicitly for a
// cold deep-link to /create/preset/:id without a prior /create visit.
void loadBundle(currentLocale(), "create");

const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const { runWithAuth } = useLockState();
const { toast } = useToast();

const presetId = String(route.params.presetId);
const preset = ref<CreatePreset | null>(null);
const presetsLoading = ref(true);

const fields = ref<Record<string, string>>({});
const revealed = ref<Record<string, boolean>>({});
const genMode = ref<GenerateMode>("random");
const generating = ref(false);
// Bumped on every generate and on lock; an in-flight generate whose token no
// longer matches is stale and must not write its result into state.
let generateToken = 0;

const submitting = ref(false);
const error = ref("");

const {
  divergence,
  resolving,
  divergeError,
  openDivergence,
  resolveDivergence,
  cancelDivergence,
} = useDivergence({
  resolveFailedKey: "create.resolveFailed",
  onResolved(result: PullResult, choice: DivergenceChoice) {
    if (choice === "adopt_remote") {
      toast.info(t("create.adoptedRemote"));
    } else {
      toast.success(t("create.keptMine", { head: result.head }));
    }
    navBack(router, { name: "entries" });
  },
  onPullFfFailed() {
    toast.info(t("create.remoteChanged"));
    navBack(router, { name: "entries" });
  },
});

onMounted(loadPreset);

async function loadPreset() {
  presetsLoading.value = true;
  try {
    const all = await listCreatePresets();
    const found = all.find((p) => p.id === presetId) ?? null;
    if (!found) {
      // Stale/unknown id (cold deep-link, preset list changed) — back to pick.
      router.replace({ name: "create" });
      return;
    }
    preset.value = found;
    fields.value = Object.fromEntries(found.fields.map((f) => [f.key, ""]));
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("create.presetsFailed");
  } finally {
    presetsLoading.value = false;
  }
}

/** Generate a password for a generatable field via the backend (CSPRNG). */
async function onGeneratePassword(f: PresetField) {
  const myToken = ++generateToken;
  generating.value = true;
  try {
    const pw = await generatePassword({
      mode: genMode.value,
      charset: f.charset,
      minLen: f.min,
      maxLen: f.max,
      strict: f.strict,
    });
    // A lock or a newer generate superseded this call — drop the result.
    if (myToken !== generateToken) return;
    fields.value[f.key] = pw;
  } catch (e) {
    if (myToken !== generateToken) return;
    const appError = e as AppError;
    toast.danger(appError?.message || t("create.genFailed"));
  } finally {
    if (myToken === generateToken) generating.value = false;
  }
}

const canSubmit = computed(() => {
  if (submitting.value || generating.value || !preset.value) return false;
  return preset.value.fields
    .filter((f) => f.required)
    .every((f) => (fields.value[f.key] ?? "").trim() !== "");
});

async function onSave() {
  if (!canSubmit.value || !preset.value) return;
  submitting.value = true;
  error.value = "";
  try {
    const outcome = await runWithAuth(() =>
      createFromPresetSecret(preset.value!.id, fields.value),
    );
    if (outcome.kind === "written") {
      toast.success(t("create.saved", { commit: outcome.commit }));
      navBack(router, { name: "entries" });
    } else if (outcome.kind === "needs_divergence_resolve") {
      const { kind: _kind, ...preview } = outcome;
      void _kind;
      openDivergence(preview);
    } else {
      error.value = t("create.saveBlocked");
    }
  } catch (e) {
    if (isAuthCancelled(e)) return;
    const appError = e as AppError;
    error.value = appError?.message || t("create.createFailed");
  } finally {
    submitting.value = false;
  }
}

// The unlock modal keeps this page mounted on auto-lock, so wipe any half-typed
// (or generated) secret the moment the identity locks, and cancel in-flight gens.
function wipeFields() {
  generateToken++;
  generating.value = false;
  revealed.value = {};
  fields.value = {};
}
useWipeOnLeave(wipeFields);

function goBack() {
  navBack(router, { name: "create" });
}
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <header class="flex items-center gap-3 mb-6" role="banner">
      <button
        @click="goBack"
        class="back-btn inline-flex items-center gap-1"
        :aria-label="t('common.back')"
      >
        <BaseIcon :icon="ArrowLeft" /> {{ t("common.back") }}
      </button>
      <h1 class="text-lg flex-1">{{ t("create.title") }}</h1>
    </header>

    <BaseAlert v-if="error" variant="danger" class="mb-3">{{
      error
    }}</BaseAlert>

    <div v-if="presetsLoading" class="loading">
      <BaseSpinner /> {{ t("create.loading") }}
    </div>

    <section v-else-if="preset">
      <p class="text-sm text-muted mb-3">
        {{ t("create.savedUnder") }} <code>{{ preset.prefix }}/…</code>
      </p>
      <form class="flex flex-col gap-4" @submit.prevent="onSave">
        <div
          v-for="f in preset.fields"
          :key="f.key"
          class="flex flex-col gap-1"
        >
          <label :for="`f-${f.key}`" class="text-sm font-medium">
            {{ f.label }}<span v-if="f.required" aria-hidden="true">*</span>
          </label>
          <div class="field-row">
            <BaseInput
              :id="`f-${f.key}`"
              v-model="fields[f.key]"
              :type="
                f.type === 'password' && !revealed[f.key] ? 'password' : 'text'
              "
              class="flex-1"
              :autocomplete="f.key === 'password' ? 'new-password' : 'off'"
              :inputmode="f.charset === '0123456789' ? 'numeric' : undefined"
              autocorrect="off"
              autocapitalize="off"
              spellcheck="false"
            />
            <select
              v-if="f.type === 'password' && f.charset == null"
              v-model="genMode"
              class="gen-select"
              :disabled="generating"
              :aria-label="t('create.passwordStyleAria')"
            >
              <option value="random">{{ t("create.genRandom") }}</option>
              <option value="memorable">{{ t("create.genMemorable") }}</option>
              <option value="xkcd">{{ t("create.genPassphrase") }}</option>
            </select>
            <button
              v-if="f.type === 'password'"
              type="button"
              class="icon-btn"
              :disabled="generating"
              :aria-label="
                revealed[f.key] ? t('create.hide') : t('create.show')
              "
              @click="revealed[f.key] = !revealed[f.key]"
            >
              <BaseIcon :icon="revealed[f.key] ? EyeOff : Eye" />
            </button>
            <button
              v-if="f.type === 'password'"
              type="button"
              class="icon-btn"
              :disabled="generating"
              :aria-label="t('create.generateAria')"
              @click="onGeneratePassword(f)"
            >
              <BaseIcon :icon="Dices" />
            </button>
          </div>
        </div>
        <BaseButton variant="primary" type="submit" :disabled="!canSubmit">{{
          submitting ? t("create.saving") : t("create.saveSecret")
        }}</BaseButton>
      </form>
    </section>

    <DivergenceModal
      context="save"
      :divergence="divergence"
      :resolving="resolving"
      :error="divergeError"
      @resolve="resolveDivergence"
      @close="cancelDivergence"
    />
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

.field-row {
  display: flex;
  gap: 0.5rem;
  align-items: stretch;
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

.loading {
  text-align: center;
  color: var(--color-muted);
  padding: 2rem 0;
}

code {
  font-family: var(--font-mono, monospace);
  font-size: 0.85em;
}
</style>
