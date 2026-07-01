<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { ref, computed, watch, onMounted, onBeforeUnmount } from "vue";
import { useRouter } from "vue-router";
import { invoke } from "@tauri-apps/api/core";
import type {
  AppError,
  ConflictChoice,
  CreatePreset,
  GenerateMode,
  PresetField,
  WriteConflict,
  WriteOutcome,
} from "../types";
import {
  isAuthCancelled,
  onLock,
  runWithAuth,
  useOverlayBackHandler,
} from "../composables";
import WriteConflictModal from "../components/WriteConflictModal.vue";
import BaseInput from "../components/base/BaseInput.vue";
import BaseTextarea from "../components/base/BaseTextarea.vue";
import BaseButton from "../components/base/BaseButton.vue";
import BaseSpinner from "../components/base/BaseSpinner.vue";
import BaseAlert from "../components/base/BaseAlert.vue";

const router = useRouter();

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
const toast = ref("");
let toastTimer: ReturnType<typeof setTimeout> | null = null;

// ── Conflict modal ────────────────────────────────────────────────────────
// The modal owns its own reveal/confirm state; this page tracks the conflict
// payload, the resolving flag, and the resolve() outcome handling.
const conflict = ref<WriteConflict | null>(null);
const resolving = ref(false);

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

// Android back while the write-conflict modal is up cancels it — same as the
// modal's Cancel button (clears the backend stash, returns to the form).
// `resolving` guards against firing mid-resolution. cancel/keep_remote need no
// identity, so this never raises the auth overlay over the modal.
useOverlayBackHandler(
  computed(() => !!conflict.value),
  () => {
    // `resolving` blocks a rapid double-back (or back right after a button
    // choice) from re-entering resolve once the stash is consumed.
    if (!resolving.value) resolve("cancel");
  },
);

function showToast(message: string) {
  toast.value = message;
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    toast.value = "";
    toastTimer = null;
  }, 3000);
}

/** Generate a password for a generatable field via the backend (CSPRNG). */
async function onGeneratePassword(f: PresetField) {
  const myToken = ++generateToken;
  generating.value = true;
  try {
    const pw = await invoke<string>("generate_password", {
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
    showToast(appError?.message || "Could not generate a password");
  } finally {
    if (myToken === generateToken) generating.value = false;
  }
}

async function loadPresets() {
  presetsLoading.value = true;
  try {
    presets.value = await invoke<CreatePreset[]>("list_create_presets");
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Failed to load presets";
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
  if (mode.value === "pick") router.push({ name: "entries" });
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
    hasTemplate.value =
      (await invoke<string | null>("lookup_template", { name })) !== null;
    preview.value = await invoke<string | null>("preview_create", {
      name,
      content: customContent.value,
    });
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
        ? invoke<WriteOutcome>("create_from_preset_secret", {
            presetId: activePreset.value.id,
            fields: fields.value,
          })
        : invoke<WriteOutcome>("create_secret", {
            name: customName.value.trim(),
            content: customContent.value,
          }),
    );

    if (outcome.kind === "written") {
      showToast(`✓ Saved (commit ${outcome.commit})`);
      router.push({ name: "entries" });
    } else {
      // Conflict — the secret is stashed backend-side; the result has no plaintext.
      // The modal resets its own reveal/confirm state when this becomes non-null.
      conflict.value = {
        name: outcome.name,
        remote_decryptable: outcome.remote_decryptable,
      };
    }
  } catch (e) {
    if (isAuthCancelled(e)) return;
    const appError = e as AppError;
    error.value = appError?.message || "Failed to create secret";
  } finally {
    submitting.value = false;
  }
}

/** Resolve the conflict with the user's choice; the backend consumes the stash. */
async function resolve(choice: ConflictChoice) {
  resolving.value = true;
  try {
    // cancel/keep_remote need no cached identity (store-side). keep_mine/_force
    // re-encrypt the stashed plaintext — deriving our own recipient + encrypting
    // requires the identity (store: encrypt_and_write → get_identity_bytes, which
    // returns IdentityEncrypted when the cache is empty), so those auth-gate.
    // Back-on-conflict only ever cancels, so the back path stays auth-free (no
    // stacking); the gate arms only for a deliberate "Keep mine".
    const needsIdentity =
      choice === "keep_mine" || choice === "keep_mine_force";
    const result = needsIdentity
      ? await runWithAuth(() =>
          invoke<{ commit: string } | null>("resolve_write_conflict", {
            choice,
          }),
        )
      : await invoke<{ commit: string } | null>("resolve_write_conflict", {
          choice,
        });
    conflict.value = null;
    if (choice === "keep_remote") {
      showToast("Kept the existing entry");
      router.push({ name: "entries" });
    } else if (choice === "cancel") {
      // Stash cleared; return to the form so the user can adjust and retry.
      showToast("Cancelled — nothing saved");
    } else if (result) {
      showToast(`✓ Saved (commit ${result.commit})`);
      router.push({ name: "entries" });
    }
  } catch (e) {
    const appError = e as AppError;
    if (appError?.code === "PUSH_REJECTED") {
      // Remote moved again mid-resolution — close the modal, retry from the form.
      conflict.value = null;
      showToast("Remote changed again — review and Save again");
    } else {
      conflict.value = null;
      showToast(appError?.message || "Could not resolve the conflict");
    }
  } finally {
    resolving.value = false;
  }
}

onMounted(loadPresets);

onBeforeUnmount(() => {
  if (previewTimer) clearTimeout(previewTimer);
  if (toastTimer) clearTimeout(toastTimer);
  // Wipe any in-form secret values (e.g. an auto-lock redirect mid-create).
  fields.value = {};
  revealed.value = {};
  customContent.value = "";
});
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <header class="flex items-center gap-3 mb-6" role="banner">
      <button @click="goBack" class="back-btn" aria-label="Back">← Back</button>
      <h1 class="text-lg flex-1">New secret</h1>
    </header>

    <BaseAlert v-if="error" variant="danger" class="mb-3">{{
      error
    }}</BaseAlert>
    <BaseAlert v-if="toast" variant="success" class="mb-3">
      {{ toast }}
    </BaseAlert>

    <!-- Step 1: pick a type -->
    <section v-if="mode === 'pick'">
      <p class="text-sm text-muted mb-3">Choose a type of secret to create.</p>
      <div v-if="presetsLoading" class="loading"><BaseSpinner /> Loading…</div>
      <ul v-else class="list-none flex flex-col gap-2" role="list">
        <li v-for="p in presets" :key="p.id">
          <button class="type-card" @click="pickPreset(p)">
            <span class="block text-base font-medium">{{ p.label }}</span>
            <span class="block text-xs text-muted"
              >Saved under {{ p.prefix }}/</span
            >
          </button>
        </li>
        <li>
          <button class="type-card" @click="pickCustom">
            <span class="block text-base font-medium">Custom secret</span>
            <span class="block text-xs text-muted"
              >Any path and content you choose</span
            >
          </button>
        </li>
      </ul>
    </section>

    <!-- Step 2a: preset form -->
    <section v-else-if="mode === 'preset' && activePreset">
      <p class="text-sm text-muted mb-3">
        Saved under <code>{{ activePreset.prefix }}/…</code>
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
              aria-label="Password style"
            >
              <option value="random">Random</option>
              <option value="memorable">Memorable</option>
              <option value="xkcd">Passphrase</option>
            </select>
            <button
              v-if="f.type === 'password'"
              type="button"
              class="icon-btn"
              :disabled="generating"
              :aria-label="revealed[f.key] ? 'Hide' : 'Show'"
              @click="revealed[f.key] = !revealed[f.key]"
            >
              {{ revealed[f.key] ? "🙈" : "👁" }}
            </button>
            <button
              v-if="f.type === 'password'"
              type="button"
              class="icon-btn"
              :disabled="generating"
              aria-label="Generate password"
              @click="onGeneratePassword(f)"
            >
              🎲
            </button>
          </div>
        </div>
        <BaseButton variant="primary" type="submit" :disabled="!canSubmit">{{
          submitting ? "Saving…" : "Save secret"
        }}</BaseButton>
      </form>
    </section>

    <!-- Step 2b: custom form -->
    <section v-else-if="mode === 'custom'">
      <form class="flex flex-col gap-4" @submit.prevent="submit">
        <div class="flex flex-col gap-1">
          <label for="c-name" class="text-sm font-medium">
            Path / name<span aria-hidden="true">*</span>
          </label>
          <BaseInput
            id="c-name"
            v-model="customName"
            type="text"
            placeholder="e.g. servers/db1"
            autocomplete="off"
          />
          <small class="text-xs text-muted"
            >First line of the content becomes the password.</small
          >
        </div>
        <div class="flex flex-col gap-1">
          <label for="c-content" class="text-sm font-medium">
            Content<span aria-hidden="true">*</span>
          </label>
          <BaseTextarea
            id="c-content"
            v-model="customContent"
            rows="4"
            autocomplete="off"
          />
        </div>
        <BaseAlert v-if="hasTemplate" variant="info">
          A <code>.pass-template</code> applies to this location — what's shown
          below is what will be stored.
        </BaseAlert>
        <pre v-if="preview" class="preview">{{ preview }}</pre>
        <BaseButton variant="primary" type="submit" :disabled="!canSubmit">{{
          submitting ? "Saving…" : "Save secret"
        }}</BaseButton>
      </form>
    </section>

    <!-- Conflict modal (shared with the edit page) -->
    <WriteConflictModal
      :conflict="conflict"
      :resolving="resolving"
      @resolve="resolve"
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
.type-card:hover {
  background: var(--color-hover);
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

.icon-btn:hover:not(:disabled) {
  background: var(--color-hover);
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
