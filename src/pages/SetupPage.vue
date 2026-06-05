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

async function onSubmit() {
  error.value = "";
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
          <small>HTTPS PAT for git authentication. Leave empty for public repos.</small>
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
  padding: 1rem;
}

.setup-card {
  width: 100%;
  max-width: 420px;
  background: white;
  border-radius: 12px;
  padding: 2rem;
  box-shadow: 0 2px 12px rgba(0, 0, 0, 0.08);
}

@media (prefers-color-scheme: dark) {
  .setup-card {
    background: #252540;
  }
}

h1 {
  text-align: center;
  font-size: 1.75rem;
  margin-bottom: 0.25rem;
}

.subtitle {
  text-align: center;
  color: #888;
  font-size: 0.875rem;
  margin-bottom: 1.5rem;
}

.setup-form {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

label {
  font-size: 0.875rem;
  font-weight: 500;
}

input, textarea {
  padding: 0.6rem 0.75rem;
  border: 1px solid #ddd;
  border-radius: 8px;
  font-size: 0.9rem;
  font-family: inherit;
  background: #fafafa;
  color: inherit;
}

input:focus, textarea:focus {
  outline: none;
  border-color: #4a6cf7;
  box-shadow: 0 0 0 2px rgba(74, 108, 247, 0.2);
}

@media (prefers-color-scheme: dark) {
  input, textarea {
    background: #1a1a2e;
    border-color: #444;
  }
}

small {
  font-size: 0.75rem;
  color: #888;
}

.error {
  background: #fee;
  color: #c33;
  padding: 0.5rem 0.75rem;
  border-radius: 6px;
  font-size: 0.85rem;
}

@media (prefers-color-scheme: dark) {
  .error {
    background: #3a1a1a;
  }
}

.btn-primary {
  padding: 0.75rem;
  background: #4a6cf7;
  color: white;
  border: none;
  border-radius: 8px;
  font-size: 1rem;
  font-weight: 500;
  cursor: pointer;
  transition: background 0.2s;
}

.btn-primary:hover:not(:disabled) {
  background: #3a5ce5;
}

.btn-primary:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}
</style>
