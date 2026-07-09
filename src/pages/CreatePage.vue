<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import {
  createFromPresetSecret,
  createSecret,
  discardDivergence,
  generatePassword,
  listCreatePresets,
  lookupTemplate,
  previewCreate,
  resolveSyncDivergence,
  type AppError,
  type CreatePreset,
  type DivergenceChoice,
  type GenerateMode,
  type PresetField,
  type SyncDivergence,
  type WriteOutcome,
} from "@/api";
import DivergenceModal from "@/components/DivergenceModal.vue";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import BaseSpinner from "@/components/base/BaseSpinner.vue";
import BaseTextarea from "@/components/base/BaseTextarea.vue";
import { isAuthCancelled, useLockState, useToast } from "@/composables";
import { navBack } from "@/utils/nav";
import { ArrowLeft, Dices, Eye, EyeOff } from "@lucide/vue";
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";

const { t } = useI18n();
const router = useRouter();
const { onLock, runWithAuth } = useLockState();
const { toast } = useToast();

// ── Presets + step ────────────────────────────────────────────────────────
const presets = ref<CreatePreset[]>([]);
const presetsLoading = ref(true);
const mode = ref<"pick" | "preset" | "custom">("pick");
const activePreset = ref<CreatePreset | null>(null);

// ── Form values ───────────────────────────────────────────────────────────
const fields = ref<Record<string, string>>({});
const customName = ref("");
const customContent = ref("");

// ── Password generator ────────────────────────────────────────────────────
const genMode = ref<GenerateMode>("random");
const generating = ref(false);
const revealed = ref<Record<string, boolean>>({});
// Bumped on every generate and on lock; an in-flight generate whose token no
// longer matches is stale and must not write its result into state.
let generateToken = 0;

// ── Template hint / live preview (custom mode) ────────────────────────────
const hasTemplate = ref(false);
const preview = ref<string | null>(null);
let previewTimer: ReturnType<typeof setTimeout> | null = null;

// ── Submission state ──────────────────────────────────────────────────────
const submitting = ref(false);
const error = ref("");

// ── Divergence modal (save-triggered) ─────────────────────────────────────
// The modal owns its own confirm step; this page tracks the divergence payload,
// the resolving flag, the resolve-error line, and the resolve() handling.
const divergence = ref<SyncDivergence | null>(null);
const resolving = ref(false);
const divergeError = ref("");

// The unlock modal keeps this page mounted on auto-lock, so wipe any half-typed
// secret the moment the identity locks.
onLock(() => {
  // Cancel any in-flight generate so its resolved promise can't repopulate a
  // secret after this wipe, and drop reveal state so fields don't reopen plain.
  generateToken++;
  generating.value = false;
  revealed.value = {};
  fields.value = {};
  customContent.value = "";
});

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

async function loadPresets() {
  presetsLoading.value = true;
  try {
    presets.value = await listCreatePresets();
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("create.presetsFailed");
  } finally {
    presetsLoading.value = false;
  }
}

function pickPreset(p: CreatePreset) {
  activePreset.value = p;
  fields.value = Object.fromEntries(p.fields.map((f) => [f.key, ""]));
  revealed.value = {};
  error.value = "";
  mode.value = "preset";
}

function pickCustom() {
  activePreset.value = null;
  customName.value = "";
  customContent.value = "";
  hasTemplate.value = false;
  preview.value = null;
  error.value = "";
  mode.value = "custom";
}

/** The generate card in the pick step routes to the standalone generator (which
 *  only copies to the clipboard — it saves nothing). Kept inside the ＋ flow
 *  because "generate a one-off password" is the same intent as "create a
 *  secret", just without persistence. */
function openGenerate() {
  router.push({ name: "generate" });
}

function backToPick() {
  mode.value = "pick";
  activePreset.value = null;
  fields.value = {};
  revealed.value = {};
  customName.value = "";
  customContent.value = "";
  hasTemplate.value = false;
  preview.value = null;
  error.value = "";
}

function goBack() {
  if (mode.value === "pick") navBack(router, { name: "entries" });
  else backToPick();
}

const canSubmit = computed(() => {
  if (submitting.value) return false;
  if (generating.value) return false;
  if (mode.value === "preset" && activePreset.value) {
    return activePreset.value.fields
      .filter((f) => f.required)
      .every((f) => (fields.value[f.key] ?? "").trim() !== "");
  }
  if (mode.value === "custom") {
    return customName.value.trim() !== "" && customContent.value.trim() !== "";
  }
  return false;
});

// Debounced template lookup + preview for custom mode (location-based, gopass).
watch([customName, customContent], () => {
  if (mode.value !== "custom") return;
  if (previewTimer) clearTimeout(previewTimer);
  previewTimer = setTimeout(refreshPreview, 200);
});

async function refreshPreview() {
  const name = customName.value.trim();
  if (name === "") {
    hasTemplate.value = false;
    preview.value = null;
    return;
  }
  try {
    hasTemplate.value = (await lookupTemplate(name)) !== null;
    preview.value = await previewCreate(name, customContent.value);
  } catch {
    // Invalid name mid-typing, or a template references an unknown var — no preview.
    hasTemplate.value = false;
    preview.value = null;
  }
}

async function submit() {
  if (!canSubmit.value) return;
  submitting.value = true;
  error.value = "";
  try {
    const outcome: WriteOutcome = await runWithAuth(() =>
      mode.value === "preset" && activePreset.value
        ? createFromPresetSecret(activePreset.value.id, fields.value)
        : createSecret(customName.value.trim(), customContent.value),
    );

    if (outcome.kind === "written") {
      toast.success(t("create.saved", { commit: outcome.commit }));
      // Pop to entries (the opener). The finished compose form becomes forward
      // history, which Android system back can't reopen.
      navBack(router, { name: "entries" });
    } else if (outcome.kind === "needs_divergence_resolve") {
      // The push lost a race — surface the divergence for the user to resolve.
      // The local commit was made; adopt discards it, keep pushes it. Drop the
      // `kind` tag so the modal gets a plain SyncDivergence.
      const { kind: _kind, ...preview } = outcome;
      void _kind;
      divergence.value = preview;
      divergeError.value = "";
    } else {
      // authenticity_blocked — the pre-write pull was refused under Enforce.
      // Stay on the form; the user resolves signatures via the Sync screen.
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

/** The user dismissed the save-triggered divergence modal (cancel / back). The
 *  local commit stays and publishes on the next Sync; clear the identity the
 *  save path kept alive (deferred wipe) for a possible keep-mine resolve. */
function cancelDivergence() {
  // Guard: a back press (BaseModalShell → emit close → cancelAll → here) could
  // re-enter after `divergence` is already cleared; only wipe once.
  if (!divergence.value) return;
  divergence.value = null;
  divergeError.value = "";
  void discardDivergence().catch(() => {});
}

/** Resolve the save-triggered divergence per the user's choice. `keep_mine`
 *  re-encrypts local-only entries onto the reviewed remote tip and pushes —
 *  that needs the identity, so it's auth-gated (runWithAuth). `adopt_remote`
 *  is a fast-forward (no identity needed). */
async function resolveDivergence(choice: DivergenceChoice) {
  if (!divergence.value) return;
  resolving.value = true;
  divergeError.value = "";
  const expectedRemoteOid = divergence.value.remote_tip;
  try {
    const result =
      choice === "keep_mine"
        ? await runWithAuth(() =>
            resolveSyncDivergence(expectedRemoteOid, choice),
          )
        : await resolveSyncDivergence(expectedRemoteOid, choice);
    divergence.value = null;
    if (choice === "adopt_remote") {
      toast.info(t("create.adoptedRemote"));
    } else {
      // keep_mine pushed the local entries — the head is now published.
      toast.success(t("create.keptMine", { head: result.head }));
    }
    navBack(router, { name: "entries" });
  } catch (e) {
    if (isAuthCancelled(e)) return;
    const appError = e as AppError;
    if (appError?.code === "PULL_FF_FAILED") {
      // The remote moved since the user reviewed — recheck from the entries list.
      divergence.value = null;
      toast.info(t("create.remoteChanged"));
      navBack(router, { name: "entries" });
    } else {
      divergeError.value = appError?.message || t("create.resolveFailed");
    }
  } finally {
    resolving.value = false;
  }
}

onMounted(loadPresets);

onBeforeUnmount(() => {
  if (previewTimer) clearTimeout(previewTimer);
  // Wipe any in-form secret values (e.g. an auto-lock redirect mid-create).
  fields.value = {};
  revealed.value = {};
  customContent.value = "";
});
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

    <!-- Step 1: pick a type -->
    <section v-if="mode === 'pick'">
      <p class="text-sm text-muted mb-3">{{ t("create.pickHint") }}</p>
      <div v-if="presetsLoading" class="loading">
        <BaseSpinner /> {{ t("create.loading") }}
      </div>
      <ul v-else class="list-none flex flex-col gap-2" role="list">
        <li v-for="p in presets" :key="p.id">
          <button class="type-card" @click="pickPreset(p)">
            <span class="block text-base font-medium">{{ p.label }}</span>
            <span class="block text-xs text-muted"
              >{{ t("create.savedUnder") }} {{ p.prefix }}/</span
            >
          </button>
        </li>
        <li>
          <button class="type-card" @click="pickCustom">
            <span class="block text-base font-medium">{{
              t("create.customLabel")
            }}</span>
            <span class="block text-xs text-muted">{{
              t("create.customHint")
            }}</span>
          </button>
        </li>
        <li>
          <button class="type-card" @click="openGenerate">
            <span class="flex items-center gap-2 text-base font-medium">
              <BaseIcon :icon="Dices" :size="18" />
              {{ t("create.generateLabel") }}
            </span>
            <span class="block text-xs text-muted">{{
              t("create.generateHint")
            }}</span>
          </button>
        </li>
      </ul>
    </section>

    <!-- Step 2a: preset form -->
    <section v-else-if="mode === 'preset' && activePreset">
      <p class="text-sm text-muted mb-3">
        {{ t("create.savedUnder") }} <code>{{ activePreset.prefix }}/…</code>
      </p>
      <form class="flex flex-col gap-4" @submit.prevent="submit">
        <div
          v-for="f in activePreset.fields"
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

    <!-- Step 2b: custom form -->
    <section v-else-if="mode === 'custom'">
      <form class="flex flex-col gap-4" @submit.prevent="submit">
        <div class="flex flex-col gap-1">
          <label for="c-name" class="text-sm font-medium">
            {{ t("create.pathName") }}<span aria-hidden="true">*</span>
          </label>
          <BaseInput
            id="c-name"
            v-model="customName"
            type="text"
            :placeholder="t('create.pathPlaceholder')"
            autocomplete="off"
          />
          <small class="text-xs text-muted">{{
            t("create.firstLineHint")
          }}</small>
        </div>
        <div class="flex flex-col gap-1">
          <label for="c-content" class="text-sm font-medium">
            {{ t("create.content") }}<span aria-hidden="true">*</span>
          </label>
          <BaseTextarea
            id="c-content"
            v-model="customContent"
            rows="4"
            autocomplete="off"
          />
        </div>
        <BaseAlert v-if="hasTemplate" variant="info">
          {{ t("create.templateHint") }}
        </BaseAlert>
        <pre v-if="preview" class="preview">{{ preview }}</pre>
        <BaseButton variant="primary" type="submit" :disabled="!canSubmit">{{
          submitting ? t("create.saving") : t("create.saveSecret")
        }}</BaseButton>
      </form>
    </section>

    <!-- Divergence modal (save-triggered — "save" wording) -->
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
.type-card {
  display: block;
  width: 100%;
  text-align: left;
  padding: 0.75rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  background: var(--color-surface);
  cursor: pointer;
  min-height: 48px;
}
.type-card:active {
  background: var(--color-hover);
}
@media (hover: hover) {
  .type-card:hover {
    background: var(--color-hover);
  }
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

.preview {
  padding: 0.5rem 0.75rem;
  background: var(--color-accent-ring);
  border-radius: var(--radius-sm);
  font-size: var(--text-sm);
  white-space: pre-wrap;
  break-all: word-break;
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
