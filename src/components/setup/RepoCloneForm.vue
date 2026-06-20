<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { computed, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { AppError, CommitIdentity } from "../../types";
import RepoAuthFields from "./RepoAuthFields.vue";
import { isSshUrl as isSshRepoUrl } from "./url";
import "./forms.css";

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
const progressStep = ref(0);
const progressSteps = ["Cloning repository..."];
let progressTimer: ReturnType<typeof setInterval> | null = null;

// Whether the current URL is an SSH remote (shared helper; RepoAuthFields also
// derives it for its own UI — same source repoUrl, so both agree).
const isSshUrl = computed(() => isSshRepoUrl(repoUrl.value));

// ── Commit identity (Advanced) ──────────────────────────────────────────
const commitName = ref("");
const commitEmail = ref("");
const commitDefault = ref<CommitIdentity | null>(null);

function startProgress() {
  progressStep.value = 0;
  progressTimer = setInterval(() => {
    if (progressStep.value < progressSteps.length - 1) {
      progressStep.value++;
    }
  }, 2000);
}

function stopProgress() {
  if (progressTimer) {
    clearInterval(progressTimer);
    progressTimer = null;
  }
}

// Fetch the default commit identity lazily when Advanced is first opened, so
// the form mount doesn't fire an extra IPC (and stays out of the test
// sequence).
async function onAdvancedToggle(e: Event) {
  if (!(e.target as HTMLDetailsElement).open || commitDefault.value) return;
  try {
    commitDefault.value = await invoke<CommitIdentity>(
      "get_commit_identity_default",
    );
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
  const validationError = validateStep1();
  if (validationError) {
    error.value = validationError;
    return;
  }

  loading.value = true;
  startProgress();

  try {
    await invoke("clone_repo", {
      repoUrl: repoUrl.value,
      pat: isSshUrl.value ? null : pat.value || null,
      sshKey: isSshUrl.value ? sshKey.value : null,
      sshPassphrase: isSshUrl.value ? sshPassphrase.value || null : null,
    });
    stopProgress();
    loading.value = false;
    // Persist a custom commit identity if the user set one under Advanced.
    // Best-effort: a failure must not block the (already-cloned) setup.
    if (commitName.value.trim() || commitEmail.value.trim()) {
      try {
        await invoke("set_commit_identity", {
          name: commitName.value.trim() || null,
          email: commitEmail.value.trim() || null,
        });
      } catch {
        // Non-critical — editable later in Settings.
      }
    }
    emit("done");
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Clone failed";
    stopProgress();
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
          <input
            id="su-commit-name"
            v-model="commitName"
            type="text"
            placeholder="Name"
            autocomplete="off"
            :disabled="loading"
            class="input-base"
          />
        </div>
        <div class="flex flex-col gap-1">
          <label for="su-commit-email" class="text-xs text-muted">Email</label>
          <input
            id="su-commit-email"
            v-model="commitEmail"
            type="email"
            placeholder="Email"
            autocomplete="off"
            :disabled="loading"
            class="input-base"
          />
        </div>
      </div>
    </details>

    <div
      v-if="error"
      class="bg-danger-soft text-danger p-2 px-3 rounded-sm text-sm"
      role="alert"
    >
      {{ error }}
    </div>

    <button type="submit" :disabled="loading" class="btn-primary">
      <span v-if="loading" class="spinner-white" aria-hidden="true"></span>
      <span v-if="loading">{{ progressSteps[progressStep] }}</span>
      <span v-else>Clone Repository</span>
    </button>
  </form>
</template>
