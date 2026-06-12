<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import { ref } from "vue";
import { useRouter } from "vue-router";
import { invoke } from "@tauri-apps/api/core";

const router = useRouter();

const passphrase = ref("");
const loading = ref(false);
const error = ref("");

async function onUnlock() {
  error.value = "";

  if (!passphrase.value) {
    error.value = "Passphrase is required";
    return;
  }

  loading.value = true;
  try {
    await invoke("unlock", { passphrase: passphrase.value });
    router.push({ name: "entries" });
  } catch (e) {
    const appError = e as { code?: string; message?: string };
    if (appError?.code === "WRONG_PASSPHRASE") {
      error.value = "Wrong passphrase — please try again";
    } else {
      error.value = appError?.message || "Unlock failed";
    }
  } finally {
    loading.value = false;
  }
}

async function onReset() {
  if (!confirm("Reset gpm? This will remove all local data and configuration."))
    return;
  try {
    await invoke("reset_config");
    router.push({ name: "setup" });
  } catch (e) {
    const appError = e as { message?: string };
    error.value = appError?.message || "Reset failed";
  }
}
</script>

<template>
  <main
    class="min-h-screen flex items-center justify-center max-[480px]:items-start p-4 max-[480px]:pt-6 max-[480px]:pb-0"
    role="main"
  >
    <div
      class="w-full max-w-[420px] bg-surface rounded-lg p-8 shadow-[0_2px_12px_rgba(0,0,0,0.08)] max-[480px]:p-4 max-[480px]:pb-[calc(3rem+4rem)]"
    >
      <h1 class="text-center text-display mb-1">🔐 gpm</h1>
      <p class="text-center text-muted text-sm mb-6">Identity is locked</p>

      <form @submit.prevent="onUnlock" class="flex flex-col gap-4">
        <div class="flex flex-col gap-1">
          <label for="passphrase" class="text-sm font-medium">Passphrase</label>
          <input
            id="passphrase"
            v-model="passphrase"
            type="password"
            placeholder="Enter your passphrase"
            required
            autocomplete="off"
            :disabled="loading"
            class="input-base"
            autofocus
          />
          <small class="text-xs text-muted"
            >Enter the passphrase to unlock your identity</small
          >
        </div>

        <div
          v-if="error"
          class="bg-danger-soft text-danger p-2 px-3 rounded-sm text-sm"
          role="alert"
        >
          {{ error }}
        </div>

        <button type="submit" :disabled="loading" class="btn-primary">
          <span v-if="loading" class="spinner-white" aria-hidden="true"></span>
          <span v-if="loading">Decrypting…</span>
          <span v-else>Unlock</span>
        </button>

        <button
          type="button"
          class="self-center text-xs text-muted hover:text-danger transition-colors"
          @click="onReset"
        >
          Reset all data
        </button>
      </form>
    </div>
  </main>
</template>

<style scoped>
.input-base {
  padding: 0.6rem 0.75rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-md);
  font-size: var(--text-base);
  font-family: inherit;
  background: var(--color-input);
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
  font-size: var(--text-md);
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

.spinner-white {
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
</style>
