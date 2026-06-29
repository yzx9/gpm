<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { ref, computed } from "vue";
import { useRoute, useRouter } from "vue-router";
import { invoke } from "@tauri-apps/api/core";
import type {
  AppError,
  ConflictChoice,
  SensitiveContent,
  WriteConflict,
  WriteOutcome,
} from "../types";
import { useSecretReveal } from "../utils/useSecretReveal";
import { isAuthCancelled, onLock, runWithAuth } from "../utils/useLockState";
import { useOverlayBackHandler } from "../utils/useOverlayBackHandler";
import { useSecuritySettings } from "../utils/useSecuritySettings";
import WriteConflictModal from "../components/WriteConflictModal.vue";

const route = useRoute();
const router = useRouter();

const entryPath = decodeURIComponent(
  Array.isArray(route.params.pathMatch)
    ? route.params.pathMatch[0]
    : route.params.pathMatch,
);
const entryName = entryPath.replace(/\.age$/, "");

// Sensitive state lives in the shared secure-reveal composable: configurable
// auto-clear, wipe on unmount, wipe on browser back. `copyPassword` calls
// `clear()` itself.
const { password, notes, revealed, reveal, clear } = useSecretReveal();
const { viewClearSecs } = useSecuritySettings();
const loading = ref(false);
const error = ref("");
const toast = ref("");
const deleting = ref(false);
let toastTimer: ReturnType<typeof setTimeout> | null = null;

// ── Edit mode ──────────────────────────────────────────────────────────────
// Edit holds the working plaintext in plain refs (not the reveal composable) so
// it can stay armed while editing without a 30s auto-clear firing mid-edit. To
// keep a single plaintext copy, entering edit drops the reveal buffer; the refs
// are wiped on save/cancel and on lock.
const editing = ref(false);
const saving = ref(false);
const editPassword = ref("");
const editNotes = ref("");
// The reassembled body captured at edit-entry, for the no-op-save dirty-check.
const loadedBody = ref("");

// ── Conflict (edit collides with a newer remote) ────────────────────────────
const conflict = ref<WriteConflict | null>(null);
const resolving = ref(false);

// Wipe edit plaintext on lock so it doesn't survive the 5-min auto-lock behind a
// wiped identity (mirrors CreatePage's onLock wipe of the compose buffer).
onLock(() => {
  exitEdit();
  conflict.value = null;
});

// Android back while the write-conflict modal is up cancels it — same as the
// modal's Cancel button (clears the backend stash, keeps the draft). `resolving`
// guards against firing mid-resolution. cancel/keep_remote need no identity, so
// this never raises the auth overlay (no stacking).
useOverlayBackHandler(
  computed(() => !!conflict.value),
  () => {
    // `resolving` blocks a rapid double-back (or back right after a button
    // choice) from re-entering resolveEdit once the stash is consumed.
    if (!resolving.value) resolveEdit("cancel");
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

async function showPassword() {
  loading.value = true;
  error.value = "";
  try {
    const result = await runWithAuth(() =>
      invoke<SensitiveContent>("show_password", { entryPath }),
    );
    reveal(result);
  } catch (e) {
    if (isAuthCancelled(e)) return;
    const appError = e as AppError;
    error.value = appError?.message || "Decryption failed";
  } finally {
    loading.value = false;
  }
}

async function copyPassword() {
  error.value = "";
  try {
    const result = await runWithAuth(() =>
      invoke<import("../types").CopyResult>("copy_password", { entryPath }),
    );
    clear();
    showToast(
      `✓ Copied ${result.entry_name} (${result.cleared_after_secs}s auto-clear)`,
    );
  } catch (e) {
    if (isAuthCancelled(e)) return;
    const appError = e as AppError;
    error.value = appError?.message || "Copy failed";
  }
}

async function deleteSecret() {
  if (deleting.value) return;
  if (
    !confirm(
      `Delete ${entryName}? This removes it everywhere on the next sync. gpm has no in-app undo — recovery is only possible via git history with external tooling.`,
    )
  ) {
    return;
  }
  deleting.value = true;
  error.value = "";
  try {
    const result = await invoke<{ commit: string }>("delete_secret", {
      name: entryName,
    });
    clear();
    showToast(`✓ Deleted (commit ${result.commit})`);
    router.push({ name: "entries" });
  } catch (e) {
    const appError = e as AppError;
    if (appError?.code === "PUSH_REJECTED") {
      // Remote diverged — delete was rolled back. Defer to the sync flow.
      showToast("Remote moved — sync to review and re-delete.");
    } else {
      error.value = appError?.message || "Delete failed";
    }
  } finally {
    deleting.value = false;
  }
}

/** Reassemble the edit body to match Secret::parse: first line is the password,
 *  the rest is notes. NO trim — Secret::parse doesn't trim the password, so
 *  trimming would silently change a secret with whitespace. Lossless inverse. */
function reassemble(pw: string, body: string): string {
  return body ? `${pw}\n${body}` : pw;
}

/** The body assembled from the current edit fields (drives the dirty-check). */
const editBody = computed(() =>
  reassemble(editPassword.value, editNotes.value),
);

/** Save is enabled only when the body has non-whitespace content and actually
 *  changed. age ciphertext is non-deterministic, so an unchanged Save would
 *  still make a spurious commit (block it); and an all-whitespace body would be
 *  rejected by `Secret::parse` on the next read, bricking the secret (block it).
 *  The trim is on the GATE only — the saved body stays untrimmed (lossless). */
const canSave = computed(
  () =>
    !saving.value &&
    editBody.value.trim() !== "" &&
    editBody.value !== loadedBody.value,
);

/** Enter edit mode. Cold-edit safe: if the user never clicked Show, fetch the
 *  content first so the fields can prefill. Drops the reveal buffer so only one
 *  plaintext copy is live. */
async function enterEdit() {
  if (editing.value) return;
  error.value = "";
  let pw = password.value;
  let nt = notes.value;
  if (pw === null) {
    // Cold edit — the page never revealed; fetch so the form can prefill.
    loading.value = true;
    try {
      const result = await invoke<SensitiveContent>("show_password", {
        entryPath,
      });
      pw = result.password;
      nt = result.notes;
    } catch (e) {
      const appError = e as AppError;
      error.value = appError?.message || "Decryption failed";
      loading.value = false;
      return;
    }
    loading.value = false;
  }
  editPassword.value = pw ?? "";
  editNotes.value = nt ?? "";
  loadedBody.value = reassemble(editPassword.value, editNotes.value);
  // Single plaintext copy: drop the read-path reveal buffer.
  clear();
  editing.value = true;
}

function exitEdit() {
  editPassword.value = "";
  editNotes.value = "";
  loadedBody.value = "";
  editing.value = false;
}

function cancelEdit() {
  exitEdit();
}

async function saveEdit() {
  if (!canSave.value) return;
  saving.value = true;
  error.value = "";
  try {
    const outcome = await invoke<WriteOutcome>("edit_secret", {
      name: entryName,
      content: editBody.value,
    });
    if (outcome.kind === "written") {
      showToast(`✓ Saved (commit ${outcome.commit})`);
      // Exit to the read-only view; the user can Show to verify. Don't
      // auto-reveal the password post-save.
      exitEdit();
    } else {
      // Conflict — the edited plaintext is stashed backend-side; the modal
      // resets its own state. Stay in edit mode with the draft intact.
      conflict.value = {
        name: outcome.name,
        remote_decryptable: outcome.remote_decryptable,
      };
    }
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Save failed";
  } finally {
    saving.value = false;
  }
}

/** Resolve an edit conflict per the user's choice; the backend consumes the
 *  stash. The modal is closed on every outcome; cancel keeps the draft. */
async function resolveEdit(choice: ConflictChoice) {
  resolving.value = true;
  try {
    // cancel/keep_remote need no cached identity; keep_mine/_force re-encrypt the
    // stashed plaintext, which needs the identity (store get_identity_bytes
    // returns IdentityEncrypted when the cache is empty), so those auth-gate.
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
      exitEdit();
    } else if (choice === "cancel") {
      // Stash cleared; stay in the edit form so the user can adjust and retry.
      showToast("Cancelled — nothing saved");
    } else if (result) {
      showToast(`✓ Saved (commit ${result.commit})`);
      exitEdit();
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

function goBack() {
  // In edit mode, Back/Escape exits edit (keeps the user on the page) instead of
  // navigating away and silently dropping the draft.
  if (editing.value) {
    cancelEdit();
    return;
  }
  clear();
  router.push({ name: "entries" });
}

function handleKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") {
    goBack();
  }
}
</script>

<template>
  <main class="max-w-[480px] mx-auto p-4" role="main" @keydown="handleKeydown">
    <header class="flex items-center gap-3 mb-6" role="banner">
      <button
        @click="goBack"
        class="bg-transparent border-none text-base cursor-pointer text-accent p-1 min-w-12 min-h-12"
        aria-label="Back to entry list"
      >
        ← Back
      </button>
      <h1
        class="text-lg whitespace-nowrap overflow-hidden text-ellipsis flex-1"
      >
        {{ entryName }}
      </h1>
    </header>

    <div
      v-if="error"
      class="bg-danger-soft text-danger p-2 px-3 rounded-sm text-sm mb-4"
      role="alert"
    >
      {{ error }}
      <span
        v-if="error.includes('ecrypt')"
        class="block text-xs opacity-80 mt-1"
      >
        Check your age identity and try again
      </span>
    </div>
    <div
      v-if="toast"
      class="bg-success-soft text-success p-2 px-3 rounded-sm text-sm mb-4"
      role="status"
      aria-live="polite"
    >
      {{ toast }}
    </div>

    <!-- Read-only view -->
    <div v-if="!editing">
      <div class="flex gap-3 mb-6">
        <button
          @click="copyPassword"
          class="btn-primary flex-1"
          :disabled="loading || deleting"
          aria-label="Copy password to clipboard"
        >
          <span aria-hidden="true">📋</span> Copy Password
        </button>
        <button
          @click="showPassword"
          class="btn-secondary flex-1"
          :disabled="loading || deleting"
          :aria-label="revealed ? 'Password is showing' : 'Show password'"
        >
          <span aria-hidden="true">👁</span>
          {{ revealed ? "Showing..." : "Show Password" }}
        </button>
      </div>

      <button
        @click="enterEdit"
        class="btn-secondary w-full mb-3"
        :disabled="loading || deleting"
        :aria-label="`Edit ${entryName}`"
      >
        ✎ Edit
      </button>

      <button
        @click="deleteSecret"
        class="btn-danger w-full mb-6"
        :disabled="deleting || loading"
        :aria-label="`Delete ${entryName}`"
      >
        {{ deleting ? "Deleting…" : "Delete" }}
      </button>

      <div v-if="loading" class="text-center text-muted py-4">
        <span class="spinner"></span>
        <span>Decrypting...</span>
      </div>

      <div
        v-if="revealed && password !== null"
        class="bg-surface rounded-lg p-4 shadow-[0_1px_6px_rgba(0,0,0,0.06)]"
      >
        <div class="mb-4">
          <label
            class="block text-xs font-semibold uppercase tracking-wide text-muted mb-1"
            >Password</label
          >
          <div
            class="font-mono text-lg p-2 bg-accent-ring rounded-sm break-all select-all"
          >
            {{ password }}
          </div>
        </div>

        <div v-if="notes" class="mb-2">
          <label
            class="block text-xs font-semibold uppercase tracking-wide text-muted mb-1"
            >Notes</label
          >
          <pre
            class="text-sm p-2 bg-input rounded-sm whitespace-pre-wrap break-all font-[inherit] select-text max-h-[200px] overflow-y-auto"
            >{{ notes }}</pre
          >
        </div>

        <p class="text-center text-xs text-muted mt-3">
          {{
            viewClearSecs > 0
              ? `Auto-clears in ${viewClearSecs}s`
              : "Stays visible until hidden or locked"
          }}
        </p>
      </div>
    </div>

    <!-- Edit form -->
    <form v-else class="flex flex-col gap-4 mb-6" @submit.prevent="saveEdit">
      <div class="flex flex-col gap-1">
        <label for="e-password" class="text-sm font-medium">Password</label>
        <input
          id="e-password"
          v-model="editPassword"
          type="text"
          class="input-base font-mono"
          autocomplete="off"
          spellcheck="false"
        />
      </div>
      <div class="flex flex-col gap-1">
        <label for="e-notes" class="text-sm font-medium">Notes</label>
        <textarea
          id="e-notes"
          v-model="editNotes"
          class="input-base"
          rows="6"
          autocomplete="off"
        />
        <small class="text-xs text-muted"
          >First line is the password; the rest is notes.</small
        >
      </div>
      <div class="flex gap-3">
        <button
          type="submit"
          class="btn-primary flex-1"
          :disabled="!canSave"
          aria-label="Save changes"
        >
          {{ saving ? "Saving…" : "Save" }}
        </button>
        <button
          type="button"
          class="btn-secondary flex-1"
          @click="cancelEdit"
          :disabled="saving"
          aria-label="Cancel edit"
        >
          Cancel
        </button>
      </div>
    </form>

    <!-- Conflict modal (shared with the create page) -->
    <WriteConflictModal
      :conflict="conflict"
      :resolving="resolving"
      @resolve="resolveEdit"
    />
  </main>
</template>

<style scoped>
.btn-primary {
  padding: 0.75rem;
  background: var(--color-accent);
  color: white;
  border: none;
  border-radius: var(--radius-md);
  font-size: var(--text-base);
  font-weight: 500;
  cursor: pointer;
  transition: background 0.2s;
  min-height: 48px;
}

.btn-primary:hover:not(:disabled) {
  background: var(--color-accent-deep);
}

.btn-primary:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.btn-secondary {
  padding: 0.75rem;
  background: var(--color-surface);
  color: var(--color-accent);
  border: 1px solid var(--color-accent);
  border-radius: var(--radius-md);
  font-size: var(--text-base);
  font-weight: 500;
  cursor: pointer;
  transition: background 0.2s;
  min-height: 48px;
}

.btn-secondary:hover:not(:disabled) {
  background: var(--color-hover);
}

.btn-secondary:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.btn-danger {
  padding: 0.75rem;
  background: var(--color-surface);
  color: var(--color-danger);
  border: 1px solid var(--color-danger);
  border-radius: var(--radius-md);
  font-size: var(--text-base);
  font-weight: 500;
  cursor: pointer;
  transition: background 0.2s;
  min-height: 48px;
}

.btn-danger:hover:not(:disabled) {
  background: var(--color-danger-soft);
}

.btn-danger:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.input-base {
  padding: 0.6rem 0.75rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  font-size: var(--text-base);
  background: var(--color-input, var(--color-surface));
  color: inherit;
  min-height: 48px;
  width: 100%;
}
.input-base:focus {
  outline: none;
  border-color: var(--color-accent);
  box-shadow: 0 0 0 2px var(--color-accent-ring);
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
</style>
