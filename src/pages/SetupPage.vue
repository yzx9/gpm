<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { ref } from "vue";
import { useRouter } from "vue-router";
import { invoke } from "@tauri-apps/api/core";
import type { AppError } from "../types";

const router = useRouter();

const repoUrl = ref("");
const pat = ref("");
const identity = ref("");
const loading = ref(false);
const error = ref("");
const progressStep = ref(0);
const progressSteps = [
  "Cloning repository...",
  "Verifying encryption...",
  "Preparing store...",
];
let progressTimer: ReturnType<typeof setInterval> | null = null;

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

function validate(): string | null {
  if (!repoUrl.value.trim()) return "Repository URL is required";
  if (!repoUrl.value.trim().startsWith("https://"))
    return "Only HTTPS URLs are supported";
  if (!identity.value.trim()) return "Age identity is required";
  if (!identity.value.trim().startsWith("AGE-SECRET-KEY-"))
    return "Identity must start with AGE-SECRET-KEY-...";
  return null;
}

async function onSubmit() {
  error.value = "";
  const validationError = validate();
  if (validationError) {
    error.value = validationError;
    return;
  }

  loading.value = true;
  startProgress();

  try {
    await invoke("setup", {
      repoUrl: repoUrl.value,
      pat: pat.value || null,
      identity: identity.value,
    });
    router.push({ name: "entries" });
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || "Setup failed";
  } finally {
    stopProgress();
    loading.value = false;
  }
}
</script>

<template>
  <main class="setup-page" role="main">
    <div class="setup-card">
      <h1>🔐 gpm</h1>
      <p class="subtitle">Age-only gopass password client</p>

      <form @submit.prevent="onSubmit" class="setup-form">
        <div class="field">
          <label for="repo-url">Git Repository URL</label>
          <input
            id="repo-url"
            v-model="repoUrl"
            type="url"
            placeholder="https://github.com/user/password-store.git"
            required
            autocomplete="off"
            :disabled="loading"
          />
        </div>

        <div class="field">
          <label for="pat">Personal Access Token</label>
          <input
            id="pat"
            v-model="pat"
            type="password"
            placeholder="Optional — for private repos"
            autocomplete="off"
            :disabled="loading"
          />
          <small
            >HTTPS PAT for git authentication. Leave empty for public
            repos.</small
          >
        </div>

        <div class="field">
          <label for="identity">Age Identity</label>
          <textarea
            id="identity"
            v-model="identity"
            placeholder="AGE-SECRET-KEY-..."
            rows="3"
            required
            autocomplete="off"
            spellcheck="false"
            :disabled="loading"
          />
          <small>Paste your age secret key (starts with AGE-SECRET-KEY-)</small>
        </div>

        <p class="trust-statement">
          Stored locally. Nothing leaves your device.
        </p>

        <div v-if="error" class="error" role="alert">{{ error }}</div>

        <button type="submit" :disabled="loading" class="btn-primary">
          <span v-if="loading" class="spinner" aria-hidden="true"></span>
          <span v-if="loading">{{ progressSteps[progressStep] }}</span>
          <span v-else>Clone &amp; Setup</span>
        </button>
      </form>
    </div>
  </main>
</template>

<style scoped>
.setup-page {
  min-height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: var(--screen-padding);
}

.setup-card {
  width: 100%;
  max-width: 420px;
  background: var(--bg-surface);
  border-radius: var(--radius-lg);
  padding: var(--space-2xl);
  box-shadow: 0 2px 12px rgba(0, 0, 0, 0.08);
}

h1 {
  text-align: center;
  font-size: var(--font-size-display);
  margin-bottom: var(--space-xs);
}

.subtitle {
  text-align: center;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  margin-bottom: var(--space-xl);
}

.setup-form {
  display: flex;
  flex-direction: column;
  gap: var(--space-lg);
}

.field {
  display: flex;
  flex-direction: column;
  gap: var(--space-xs);
}

label {
  font-size: var(--font-size-sm);
  font-weight: var(--font-weight-medium);
}

input,
textarea {
  padding: 0.6rem var(--space-md);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  font-size: var(--font-size-base);
  font-family: inherit;
  background: var(--bg-input);
  color: inherit;
  min-height: var(--input-min-height);
}

input:focus,
textarea:focus {
  outline: none;
  border-color: var(--accent);
  box-shadow: 0 0 0 2px var(--accent-focus-ring);
}

small {
  font-size: var(--font-size-xs);
  color: var(--text-secondary);
}

.error {
  background: var(--danger-bg);
  color: var(--danger);
  padding: var(--space-sm) var(--space-md);
  border-radius: var(--radius-sm);
  font-size: var(--font-size-sm);
}

.trust-statement {
  text-align: center;
  font-size: var(--font-size-xs);
  color: var(--trust-color);
  background: var(--trust-bg);
  padding: var(--space-sm) var(--space-md);
  border-radius: var(--trust-radius);
}

.btn-primary {
  padding: var(--space-md);
  background: var(--accent);
  color: white;
  border: none;
  border-radius: var(--radius-md);
  font-size: var(--font-size-md);
  font-weight: var(--font-weight-medium);
  cursor: pointer;
  transition: background 0.2s;
  min-height: var(--btn-min-height);
}

.btn-primary:hover:not(:disabled) {
  background: var(--accent-hover);
}

.btn-primary:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.spinner {
  display: inline-block;
  width: 14px;
  height: 14px;
  border: 2px solid rgba(255, 255, 255, 0.3);
  border-top-color: white;
  border-radius: 50%;
  animation: spin 0.6s linear infinite;
  margin-right: 0.4rem;
  vertical-align: middle;
}

@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}

@media (max-width: 480px) {
  .setup-page {
    align-items: flex-start;
    padding-top: var(--space-xl);
    padding-bottom: 0;
  }

  .setup-card {
    padding: var(--space-lg);
    padding-bottom: calc(
      var(--btn-min-height) + var(--space-xl) + env(safe-area-inset-bottom, 0px)
    );
  }
}
</style>
