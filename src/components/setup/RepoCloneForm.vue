<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { computed, ref, onBeforeUnmount } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  cancelGit,
  cloneRepo,
  getCommitIdentityDefault,
  setCommitIdentity,
  subscribeGitProgress,
  type AppError,
  type CommitIdentity,
  type GitProgressEvent,
} from "@/api";
import RepoAuthFields from "./RepoAuthFields.vue";
import { isSshUrl as isSshRepoUrl } from "./url";
import BaseInput from "@/components/base/BaseInput.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseAlert from "@/components/base/BaseAlert.vue";

const emit = defineEmits<{
  done: [];
}>();

// Auth fields are owned by CloneFlow (hoisted) so they survive the 1↔2 step
// transition — needed for IdentitySetupForm's "Use my SSH key for decryption".
const repoUrl = defineModel<string>("repoUrl", { required: true });
const pat = defineModel<string>("pat", { required: true });
const sshKey = defineModel<string>("sshKey", { required: true });
const sshPassphrase = defineModel<string>("sshPassphrase", { required: true });

const loading = ref(false);
const error = ref("");
/** Set when the user cancelled the clone — shown as a neutral status, not a
 * red error. Cleared on the next clone attempt. */
const cancelled = ref(false);
const progressText = ref("");
const progressPercent = ref(0);
const receivedBytes = ref(0);
let progressUnlisten: UnlistenFn | null = null;

// Whether the current URL is an SSH remote (shared helper; RepoAuthFields also
// derives it for its own UI — same source repoUrl, so both agree).
const isSshUrl = computed(() => isSshRepoUrl(repoUrl.value));

// ── Commit identity (Advanced) ──────────────────────────────────────────
const commitName = ref("");
const commitEmail = ref("");
const commitDefault = ref<CommitIdentity | null>(null);

function formatBytes(n: number): string {
  if (n <= 0) return "";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

function onProgress(p: GitProgressEvent) {
  receivedBytes.value = p.received_bytes;
  if (p.total_objects > 0) {
    progressPercent.value = Math.min(
      100,
      Math.round((p.received_objects / p.total_objects) * 100),
    );
  }
  progressText.value =
    p.message ?? `${p.received_objects} / ${p.total_objects} objects`;
}

/** User-initiated cancel: flip the backend token so git2 aborts the transfer. */
async function cancelClone() {
  try {
    await cancelGit();
  } catch {
    // best-effort — the clone simply continues if cancel fails
  }
}

onBeforeUnmount(() => {
  progressUnlisten?.();
  progressUnlisten = null;
});

// Fetch the default commit identity lazily when Advanced is first opened, so
// the form mount doesn't fire an extra IPC (and stays out of the test
// sequence).
async function onAdvancedToggle(e: Event) {
  if (!(e.target as HTMLDetailsElement).open || commitDefault.value) return;
  try {
    commitDefault.value = await getCommitIdentityDefault();
  } catch {
    // Non-critical — the default hint just won't render.
  }
}

function validateStep1(): string | null {
  if (!repoUrl.value.trim()) return "Repository URL is required";
  const url = repoUrl.value.trim();
  const isHttps = url.startsWith("https://");
  const isSsh = isSshRepoUrl(url);
  if (!isHttps && !isSsh) {
    return "URL must be HTTPS or SSH format (e.g. git@host:user/repo.git)";
  }
  if (isSsh && !sshKey.value.trim()) {
    return "SSH private key is required for SSH URLs";
  }
  return null;
}

async function onClone() {
  error.value = "";
  cancelled.value = false;
  const validationError = validateStep1();
  if (validationError) {
    error.value = validationError;
    return;
  }

  loading.value = true;
  progressText.value = "Cloning repository…";
  progressPercent.value = 0;
  receivedBytes.value = 0;
  progressUnlisten ??= await subscribeGitProgress(onProgress);

  try {
    await cloneRepo(
      repoUrl.value,
      isSshUrl.value ? null : pat.value || null,
      isSshUrl.value ? sshKey.value : null,
      isSshUrl.value ? sshPassphrase.value || null : null,
    );
    // Persist a custom commit identity if the user set one under Advanced.
    // Best-effort: a failure must not block the (already-cloned) setup.
    if (commitName.value.trim() || commitEmail.value.trim()) {
      try {
        await setCommitIdentity(
          commitName.value.trim() || null,
          commitEmail.value.trim() || null,
        );
      } catch {
        // Non-critical — editable later in Settings.
      }
    }
    emit("done");
  } catch (e) {
    const appError = e as AppError;
    if (appError?.code === "CANCELLED") {
      // User-initiated abort — surface as a neutral status, not an error.
      cancelled.value = true;
    } else {
      error.value = appError?.message || "Clone failed";
    }
  } finally {
    progressUnlisten?.();
    progressUnlisten = null;
    loading.value = false;
  }
}
</script>

<template>
  <form class="flex flex-col gap-4" @submit.prevent="onClone">
    <RepoAuthFields
      v-model:repo-url="repoUrl"
      v-model:pat="pat"
      v-model:ssh-key="sshKey"
      v-model:ssh-passphrase="sshPassphrase"
      v-model:error="error"
      :show-keygen="true"
      :disabled="loading"
    />

    <!-- Advanced: commit identity -->
    <details @toggle="onAdvancedToggle">
      <summary class="text-sm text-muted cursor-pointer select-none">
        Advanced Settings
      </summary>
      <div class="flex flex-col gap-3 mt-3">
        <p class="text-xs text-muted">
          Name and email written to each git commit. Leave blank to use the
          default<span v-if="commitDefault">
            ({{ commitDefault.name }} &lt;{{ commitDefault.email }}&gt;)</span
          >.
        </p>
        <div class="flex flex-col gap-1">
          <label for="su-commit-name" class="text-xs text-muted">Name</label>
          <BaseInput
            id="su-commit-name"
            v-model="commitName"
            type="text"
            placeholder="Name"
            autocomplete="off"
            :disabled="loading"
          />
        </div>
        <div class="flex flex-col gap-1">
          <label for="su-commit-email" class="text-xs text-muted">Email</label>
          <BaseInput
            id="su-commit-email"
            v-model="commitEmail"
            type="email"
            placeholder="Email"
            autocomplete="off"
            :disabled="loading"
          />
        </div>
      </div>
    </details>

    <!-- Real clone progress + cancel -->
    <div v-if="loading" class="flex flex-col gap-1">
      <div class="flex justify-between items-center text-xs text-muted">
        <span aria-live="polite">{{ progressText }}</span>
        <button type="button" class="cancel-link" @click="cancelClone">
          Cancel
        </button>
      </div>
      <div
        class="progress-track"
        role="progressbar"
        :aria-valuenow="progressPercent"
        aria-valuemin="0"
        aria-valuemax="100"
      >
        <div
          class="progress-fill"
          :style="{ width: `${progressPercent}%` }"
        ></div>
      </div>
      <div v-if="formatBytes(receivedBytes)" class="text-xs text-subtle">
        {{ formatBytes(receivedBytes) }} received
      </div>
    </div>

    <div v-if="cancelled" class="text-sm text-muted" role="status">
      Clone cancelled.
    </div>

    <BaseAlert v-if="error" variant="danger">{{ error }}</BaseAlert>

    <BaseButton variant="primary" type="submit" :loading="loading">{{
      loading ? "Cloning…" : "Clone Repository"
    }}</BaseButton>
  </form>
</template>

<style scoped>
.progress-track {
  height: 6px;
  background: var(--color-edge);
  border-radius: 9999px;
  overflow: hidden;
}
.progress-fill {
  height: 100%;
  background: var(--color-accent);
  border-radius: 9999px;
  transition: width 0.2s ease;
}
.cancel-link {
  background: none;
  border: none;
  padding: 0;
  font: inherit;
  color: var(--color-accent);
  cursor: pointer;
}
</style>
