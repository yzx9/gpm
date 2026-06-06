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
    loading.value = false;
  }
}
</script>

<template>
  <div class="setup-page">
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
            :disabled="loading"
          />
          <small>Paste your age secret key (starts with AGE-SECRET-KEY-)</small>
        </div>

        <div v-if="error" class="error">{{ error }}</div>

        <button type="submit" :disabled="loading" class="btn-primary">
          <span v-if="loading" class="spinner"></span>
          {{ loading ? "Cloning repository..." : "Clone & Setup" }}
        </button>
      </form>
    </div>
  </div>
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
</style>
