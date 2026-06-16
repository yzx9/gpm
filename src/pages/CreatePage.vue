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
  SensitiveContent,
  WriteConflict,
  WriteOutcome,
} from "../types";
import { useSecretReveal } from "../utils/useSecretReveal";

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
const conflict = ref<WriteConflict | null>(null);
const resolving = ref(false);
const confirmForce = ref(false);
// "View existing" reveals the remote copy under the same secure-reveal contract
// as the entry detail view (30s auto-clear, wipe on unmount / back).
const {
  password: remotePw,
  notes: remoteNotes,
  revealed: remoteRevealed,
  reveal: revealRemote,
  clear: clearRemote,
} = useSecretReveal();

function showToast(message: string) {
  toast.value = message;
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    toast.value = "";
    toastTimer = null;
  }, 3000);
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
    const outcome: WriteOutcome =
      mode.value === "preset" && activePreset.value
        ? await invoke<WriteOutcome>("create_from_preset_secret", {
            presetId: activePreset.value.id,
            fields: fields.value,
          })
        : await invoke<WriteOutcome>("create_secret", {
            name: customName.value.trim(),
            content: customContent.value,
          });

    if (outcome.kind === "written") {
      showToast(`✓ Saved (commit ${outcome.commit})`);
      router.push({ name: "entries" });
    } else {
      // Conflict — the secret is stashed backend-side; the result has no plaintext.
      conflict.value = {
        name: outcome.name,
        remote_decryptable: outcome.remote_decryptable,
      };
      confirmForce.value = false;
      clearRemote();
    }
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Failed to create secret";
  } finally {
    submitting.value = false;
  }
}

/** Reveal the existing remote version (decryptable branch only). */
async function viewExisting() {
  if (!conflict.value) return;
  try {
    const content = await invoke<SensitiveContent>("show_password", {
      entryPath: conflict.value.name,
    });
    revealRemote(content);
  } catch (e) {
    const appError = e as AppError;
    showToast(appError?.message || "Could not read the existing entry");
  }
}

/** Resolve the conflict with the user's choice; the backend consumes the stash. */
async function resolve(choice: ConflictChoice) {
  resolving.value = true;
  try {
    const result = await invoke<{ commit: string } | null>(
      "resolve_write_conflict",
      { choice },
    );
    conflict.value = null;
    clearRemote();
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
  customContent.value = "";
});
</script>

<template>
  <main class="max-w-[480px] md:max-w-[600px] mx-auto p-4" role="main">
    <header class="flex items-center gap-3 mb-6" role="banner">
      <button @click="goBack" class="back-btn" aria-label="Back">← Back</button>
      <h1 class="text-lg flex-1">New secret</h1>
    </header>

    <div v-if="error" class="alert-error" role="alert">{{ error }}</div>
    <div v-if="toast" class="alert-toast" role="status" aria-live="polite">
      {{ toast }}
    </div>

    <!-- Step 1: pick a type -->
    <section v-if="mode === 'pick'">
      <p class="text-sm text-muted mb-3">Choose a type of secret to create.</p>
      <div v-if="presetsLoading" class="loading">
        <span class="spinner" /> Loading…
      </div>
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
          <input
            :id="`f-${f.key}`"
            v-model="fields[f.key]"
            type="text"
            class="input-base"
            :autocomplete="f.key === 'password' ? 'new-password' : 'off'"
          />
        </div>
        <button type="submit" class="btn-primary" :disabled="!canSubmit">
          {{ submitting ? "Saving…" : "Save secret" }}
        </button>
      </form>
    </section>

    <!-- Step 2b: custom form -->
    <section v-else-if="mode === 'custom'">
      <form class="flex flex-col gap-4" @submit.prevent="submit">
        <div class="flex flex-col gap-1">
          <label for="c-name" class="text-sm font-medium">
            Path / name<span aria-hidden="true">*</span>
          </label>
          <input
            id="c-name"
            v-model="customName"
            type="text"
            class="input-base"
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
          <textarea
            id="c-content"
            v-model="customContent"
            class="input-base"
            rows="4"
            autocomplete="off"
          />
        </div>
        <div v-if="hasTemplate" class="alert-info">
          A <code>.pass-template</code> applies to this location — what's shown
          below is what will be stored.
        </div>
        <pre v-if="preview" class="preview">{{ preview }}</pre>
        <button type="submit" class="btn-primary" :disabled="!canSubmit">
          {{ submitting ? "Saving…" : "Save secret" }}
        </button>
      </form>
    </section>

    <!-- Conflict modal -->
    <div
      v-if="conflict"
      class="overlay"
      role="dialog"
      aria-modal="true"
      aria-label="Remote copy exists"
    >
      <div class="modal-card w-full max-w-[480px]">
        <h2 class="text-base font-medium mb-1">Remote copy exists</h2>
        <p class="text-xs text-muted mb-3">
          <code>{{ conflict.name }}</code> already exists on the remote with a
          different version. Your new entry was not saved.
        </p>

        <!-- Decryptable: the user can inspect the remote and choose freely. -->
        <template v-if="conflict.remote_decryptable">
          <p class="text-sm mb-3">
            You can read the existing version (it's encrypted to you too).
          </p>
          <div
            v-if="remoteRevealed && remotePw !== null"
            class="reveal-box mb-3"
          >
            <div class="mb-2">
              <span class="block text-xs text-muted mb-1"
                >Existing password</span
              >
              <span class="font-mono break-all">{{ remotePw }}</span>
            </div>
            <div v-if="remoteNotes">
              <span class="block text-xs text-muted mb-1">Existing notes</span>
              <pre class="text-sm whitespace-pre-wrap break-all">{{
                remoteNotes
              }}</pre>
            </div>
            <p class="text-xs text-muted mt-2">Auto-clears in 30 seconds</p>
          </div>
          <button v-else class="btn-sm w-full mb-3" @click="viewExisting">
            View existing
          </button>
          <div class="flex flex-col gap-2">
            <button
              class="btn-sm"
              :disabled="resolving"
              @click="resolve('keep_mine')"
            >
              Keep mine (overwrite remote)
            </button>
            <button
              class="btn-sm"
              :disabled="resolving"
              @click="resolve('keep_remote')"
            >
              Keep existing
            </button>
            <button
              class="btn-sm"
              :disabled="resolving"
              @click="resolve('cancel')"
            >
              Cancel
            </button>
          </div>
        </template>

        <!-- Undecryptable: we can't read it; overwriting destroys unreadable data. -->
        <template v-else>
          <p class="alert-warn mb-3">
            The existing entry is encrypted for someone else — you can't read
            it. Keeping yours overwrites and destroys that unreadable version.
          </p>
          <label class="confirm-row mb-3">
            <input v-model="confirmForce" type="checkbox" />
            <span class="text-sm"
              >I understand this destroys the version I can't read.</span
            >
          </label>
          <div class="flex flex-col gap-2">
            <button
              class="btn-danger"
              :disabled="!confirmForce || resolving"
              @click="resolve('keep_mine_force')"
            >
              Keep mine anyway
            </button>
            <button
              class="btn-sm"
              :disabled="resolving"
              @click="resolve('keep_remote')"
            >
              Keep existing
            </button>
            <button
              class="btn-sm"
              :disabled="resolving"
              @click="resolve('cancel')"
            >
              Cancel
            </button>
          </div>
        </template>
      </div>
    </div>
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

.input-base {
  padding: 0.6rem 0.75rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  font-size: var(--text-base);
  background: var(--color-surface);
  color: inherit;
  min-height: 48px;
}
.input-base:focus {
  outline: none;
  border-color: var(--color-accent);
  box-shadow: 0 0 0 2px var(--color-accent-ring);
}

.btn-primary {
  padding: 0.75rem;
  background: var(--color-accent);
  color: white;
  border: none;
  border-radius: var(--radius-md);
  font-size: var(--text-base);
  font-weight: 500;
  cursor: pointer;
  min-height: 48px;
}
.btn-primary:hover:not(:disabled) {
  background: var(--color-accent-deep);
}
.btn-primary:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.btn-sm {
  padding: 0.5rem 0.75rem;
  font-size: var(--text-sm);
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-sm);
  background: var(--color-surface);
  color: inherit;
  cursor: pointer;
  min-height: 48px;
}
.btn-sm:hover:not(:disabled) {
  background: var(--color-hover);
}
.btn-sm:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.btn-danger {
  padding: 0.5rem 0.75rem;
  font-size: var(--text-sm);
  border: 1px solid var(--color-danger);
  color: var(--color-danger);
  border-radius: var(--radius-sm);
  background: var(--color-surface);
  cursor: pointer;
  min-height: 48px;
}
.btn-danger:hover:not(:disabled) {
  background: var(--color-danger-soft);
}
.btn-danger:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.alert-error {
  background: var(--color-danger-soft);
  color: var(--color-danger);
  padding: 0.5rem 0.75rem;
  border-radius: var(--radius-sm);
  font-size: var(--text-sm);
  margin-bottom: 0.75rem;
}
.alert-toast {
  background: var(--color-success-soft);
  color: var(--color-success);
  padding: 0.5rem 0.75rem;
  border-radius: var(--radius-sm);
  font-size: var(--text-sm);
  margin-bottom: 0.75rem;
}
.alert-info {
  background: var(--color-info-soft);
  color: var(--color-info);
  padding: 0.5rem 0.75rem;
  border-radius: var(--radius-sm);
  font-size: var(--text-sm);
}
.alert-warn {
  background: var(--color-warning-soft);
  color: var(--color-warning);
  padding: 0.5rem 0.75rem;
  border-radius: var(--radius-sm);
  font-size: var(--text-sm);
}

.preview {
  padding: 0.5rem 0.75rem;
  background: var(--color-accent-ring);
  border-radius: var(--radius-sm);
  font-size: var(--text-sm);
  white-space: pre-wrap;
  break-all: word-break;
}

.reveal-box {
  padding: 0.75rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  background: var(--color-surface);
}

.confirm-row {
  display: flex;
  align-items: flex-start;
  gap: 0.5rem;
}

.modal-card {
  padding: 1rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  background: var(--color-surface);
}
.overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.4);
  z-index: 40;
  display: flex;
  align-items: flex-end;
  justify-content: center;
  padding: 1rem;
}
@media (min-width: 640px) {
  .overlay {
    align-items: center;
  }
}

.loading {
  text-align: center;
  color: var(--color-muted);
  padding: 2rem 0;
}

.spinner {
  display: inline-block;
  width: 18px;
  height: 18px;
  border: 2px solid var(--color-edge);
  border-top-color: var(--color-accent);
  border-radius: 50%;
  animation: spin 0.6s linear infinite;
  margin-right: 0.5rem;
  vertical-align: middle;
}

code {
  font-family: var(--font-mono, monospace);
  font-size: 0.85em;
}
</style>
