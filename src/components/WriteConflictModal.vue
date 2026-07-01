<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { ConflictChoice, SensitiveContent, WriteConflict } from "../types";
import { useSecretReveal } from "../utils/useSecretReveal";
import BaseButton from "./base/BaseButton.vue";

const props = defineProps<{
  /** The conflict to show, or null to render nothing. */
  conflict: WriteConflict | null;
  /** True while the host is resolving (disables the choice buttons). */
  resolving: boolean;
}>();

const emit = defineEmits<{
  (e: "resolve", choice: ConflictChoice): void;
}>();

// "View existing" reveals the remote copy under the same secure-reveal contract
// as the entry detail view (30s auto-clear, wipe on unmount / lock).
const {
  password: remotePw,
  notes: remoteNotes,
  revealed: remoteRevealed,
  reveal: revealRemote,
  clear: clearRemote,
} = useSecretReveal();

const confirmForce = ref(false);

// Reset the modal's local state whenever the conflict changes — both when a new
// conflict arrives (fresh confirm + reveal) and when it clears (drop the reveal
// immediately rather than waiting for the 30s auto-clear).
watch(
  () => props.conflict,
  () => {
    confirmForce.value = false;
    clearRemote();
  },
);

/** Reveal the existing REMOTE version (decryptable branch only). Best-effort.
 *  Uses show_remote_secret (the origin tip), NOT show_password — on a conflict
 *  the local copy has been rolled back to the pre-edit version, which would
 *  mislead the user into previewing their own old copy instead of the teammate's
 *  version they're deciding whether to overwrite. */
async function viewExisting() {
  if (!props.conflict) return;
  try {
    const content = await invoke<SensitiveContent | null>(
      "show_remote_secret",
      { name: props.conflict.name },
    );
    if (content) revealRemote(content);
    else clearRemote();
  } catch {
    // The conflict already promised the remote is decryptable; if the read still
    // fails, just leave the reveal hidden — the user can still pick a choice.
    clearRemote();
  }
}
</script>

<template>
  <div
    v-if="conflict"
    class="overlay"
    role="dialog"
    aria-modal="true"
    aria-label="Remote copy exists"
  >
    <div class="modal-card w-full max-w-120">
      <h2 class="text-base font-medium mb-1">Remote copy exists</h2>
      <p class="text-xs text-muted mb-3">
        <code>{{ conflict.name }}</code> already exists on the remote with a
        different version. Your entry was not saved.
      </p>

      <!-- Decryptable: the user can inspect the remote and choose freely. -->
      <template v-if="conflict.remote_decryptable">
        <p class="text-sm mb-3">
          You can read the existing version (it's encrypted to you too).
        </p>
        <div v-if="remoteRevealed && remotePw !== null" class="reveal-box mb-3">
          <div class="mb-2">
            <span class="block text-xs text-muted mb-1">Existing password</span>
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
        <BaseButton
          v-else
          variant="secondary"
          size="sm"
          block
          class="mb-3"
          @click="viewExisting"
        >
          View existing
        </BaseButton>
        <div class="flex flex-col gap-2">
          <BaseButton
            variant="secondary"
            size="sm"
            :disabled="resolving"
            @click="emit('resolve', 'keep_mine')"
          >
            Keep mine (overwrite remote)
          </BaseButton>
          <BaseButton
            variant="secondary"
            size="sm"
            :disabled="resolving"
            @click="emit('resolve', 'keep_remote')"
          >
            Keep existing
          </BaseButton>
          <BaseButton
            variant="secondary"
            size="sm"
            :disabled="resolving"
            @click="emit('resolve', 'cancel')"
          >
            Cancel
          </BaseButton>
        </div>
      </template>

      <!-- Undecryptable: we can't read it; overwriting destroys unreadable data. -->
      <template v-else>
        <p class="alert-warn mb-3">
          The existing entry is encrypted for someone else — you can't read it.
          Keeping yours overwrites and destroys that unreadable version.
        </p>
        <label class="confirm-row mb-3">
          <input v-model="confirmForce" type="checkbox" />
          <span class="text-sm"
            >I understand this destroys the version I can't read.</span
          >
        </label>
        <div class="flex flex-col gap-2">
          <BaseButton
            variant="danger"
            size="sm"
            :disabled="!confirmForce || resolving"
            @click="emit('resolve', 'keep_mine_force')"
          >
            Keep mine anyway
          </BaseButton>
          <BaseButton
            variant="secondary"
            size="sm"
            :disabled="resolving"
            @click="emit('resolve', 'keep_remote')"
          >
            Keep existing
          </BaseButton>
          <BaseButton
            variant="secondary"
            size="sm"
            :disabled="resolving"
            @click="emit('resolve', 'cancel')"
          >
            Cancel
          </BaseButton>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
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

.modal-card {
  padding: 1rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  background: var(--color-surface);
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

.alert-warn {
  background: var(--color-warning-soft);
  color: var(--color-warning);
  padding: 0.5rem 0.75rem;
  border-radius: var(--radius-sm);
  font-size: var(--text-sm);
}

code {
  font-family: var(--font-mono, monospace);
  font-size: 0.85em;
}
</style>
