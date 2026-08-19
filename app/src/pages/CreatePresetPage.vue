<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

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
import BaseHeader from "@/components/base/BaseHeader.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import BaseSelect from "@/components/base/BaseSelect.vue";
import BaseSpinner from "@/components/base/BaseSpinner.vue";
import DivergenceModal from "@/components/DivergenceModal.vue";
import {
  isAuthCancelled,
  useCancellableSave,
  useDivergence,
  useLockState,
  useSecureClaim,
  useToast,
  useWipeOnLeave,
} from "@/composables";
import { currentLocale, loadBundle } from "@/i18n";
import { navBack } from "@/utils/nav";
import { Dices, Eye, EyeOff } from "@lucide/vue";
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

const genModeOptions = computed<{ label: string; value: GenerateMode }[]>(
  () => [
    { label: t("create.genRandom"), value: "random" },
    { label: t("create.genMemorable"), value: "memorable" },
    { label: t("create.genPassphrase"), value: "xkcd" },
  ],
);

function onGenModeChange(next: GenerateMode) {
  genMode.value = next;
}
const generating = ref(false);
// Bumped on every generate and on lock; an in-flight generate whose token no
// longer matches is stale and must not write its result into state.
let generateToken = 0;

const submitting = ref(false);
const error = ref("");
// Caught backend error code — renders the alert as a warning (not red danger)
// for PLUGIN_UNAVAILABLE (a known platform limitation, not a transient failure).
const errorCode = ref<string | null>(null);
const { cancelling, cancelSave } = useCancellableSave();

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

// R031: this form authors/generates secrets, so hold a screen-capture claim for
// the page's lifetime (released at unmount via onScopeDispose).
const { acquire: acquireSecure } = useSecureClaim();
onMounted(() => {
  void acquireSecure();
});

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
    console.warn("[create-preset] presets load failed", e);
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
    console.warn("[create-preset] field generate failed", e);
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
  errorCode.value = null;
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
    } else if (outcome.kind === "cancelled") {
      toast.info(
        outcome.committed
          ? t("create.saveCancelledLocalStays")
          : t("create.saveCancelledNothingPublished"),
      );
    } else {
      error.value = t("create.saveBlocked");
    }
  } catch (e) {
    if (isAuthCancelled(e)) return;
    const appError = e as AppError;
    errorCode.value = appError?.code ?? null;
    error.value = appError?.message || t("create.createFailed");
    console.warn("[create-preset] create failed", e);
  } finally {
    submitting.value = false;
    cancelling.value = false;
  }
}

// The unlock modal keeps this page mounted on auto-lock, so wipe any half-typed
// (or generated) secret the moment either lock fires, and cancel in-flight
// gens. Returns whether a draft was actually lost, so the lock path marks the
// drafts notice for the post-unlock toast — value-based, because loadPreset
// pre-seeds every field key with "" (a merely opened page holds nothing).
function wipeFields(): boolean {
  generateToken++;
  generating.value = false;
  revealed.value = {};
  const hadDraft = Object.values(fields.value).some((v) => v !== "");
  fields.value = {};
  return hadDraft;
}
useWipeOnLeave(wipeFields);
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <BaseHeader :back-fallback="{ name: 'create' }">
      <template #title>
        <h1 class="text-lg flex-1">{{ t("create.title") }}</h1>
      </template>
    </BaseHeader>

    <BaseAlert
      v-if="error"
      :variant="errorCode === 'PLUGIN_UNAVAILABLE' ? 'warning' : 'danger'"
      class="mb-3"
    >
      {{ error }}
    </BaseAlert>

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
            <div
              v-if="f.type === 'password' && f.charset == null"
              class="gen-mode-picker"
            >
              <BaseSelect
                :name="`gen-mode-${f.key}`"
                :aria-label="t('create.passwordStyleAria')"
                :model-value="genMode"
                :options="genModeOptions"
                :disabled="generating"
                @change="onGenModeChange"
              />
            </div>
            <BaseButton
              v-if="f.type === 'password'"
              variant="secondary"
              size="sm"
              :disabled="generating"
              :aria-label="
                revealed[f.key] ? t('create.hide') : t('create.show')
              "
              @click="revealed[f.key] = !revealed[f.key]"
            >
              <BaseIcon :icon="revealed[f.key] ? EyeOff : Eye" />
            </BaseButton>
            <BaseButton
              v-if="f.type === 'password'"
              variant="secondary"
              size="sm"
              :disabled="generating"
              :aria-label="t('create.generateAria')"
              @click="onGeneratePassword(f)"
            >
              <BaseIcon :icon="Dices" />
            </BaseButton>
          </div>
        </div>
        <div class="flex gap-3">
          <BaseButton
            variant="primary"
            type="submit"
            class="flex-1"
            :disabled="!canSubmit"
            >{{ submitting ? t("create.saving") : t("create.saveSecret") }}
          </BaseButton>
          <BaseButton
            v-if="submitting"
            variant="outline"
            type="button"
            class="flex-1"
            :disabled="cancelling"
            :aria-label="t('create.cancelSaveAria')"
            @click="cancelSave"
            >{{
              cancelling ? t("create.cancellingSave") : t("create.cancelSave")
            }}
          </BaseButton>
        </div>
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
.field-row {
  display: flex;
  gap: 0.5rem;
  align-items: stretch;
}

.gen-mode-picker {
  /* Fixed width so the picker doesn't grow/shrink as the selected label changes. */
  flex: 0 0 auto;
  width: 8.5rem;
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
