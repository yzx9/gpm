<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import type { AppConfig, AppError, LockMode, RepoConfig } from "@/api";
import { getAppConfig, getAuthState, getConfig } from "@/api";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseCard from "@/components/base/BaseCard.vue";
import BaseHeader from "@/components/base/BaseHeader.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import {
  ChevronRight,
  Database,
  KeyRound,
  Lock,
  Settings,
  SlidersHorizontal,
} from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";

const router = useRouter();
const { t } = useI18n();

// The hub is a menu of category rows. It loads just enough config to show a
// one-line current-value summary per row (the iOS settings pattern), so the
// user can verify state — language, lock mode, identity status, repo host — at
// a glance without drilling in. Each value degrades gracefully (empty) if its
// source hasn't resolved.
const appConfig = ref<AppConfig | null>(null);
const repoConfig = ref<RepoConfig | null>(null);
const identityEncrypted = ref(false);
const identityType = ref("");
// Starts true so the first paint is the spinner, not a one-frame flash of the
// default-value rows (loadSummaries runs in onMounted, after first render).
const loading = ref(true);
const error = ref("");

function lockModeLabel(mode: LockMode | undefined): string {
  if (!mode) return t("settings.lock.immediate");
  if (mode === "immediate") return t("settings.lock.immediate");
  if (mode === "never") return t("settings.lock.never");
  return t("settings.lock.minutes", { count: Math.round(mode.idle / 60) });
}

/** Strip scheme and trailing `.git` from a remote URL for a compact host/path
 *  summary (e.g. `git@github.com:user/repo.git` → `github.com:user/repo`). */
function repoHost(url: string): string {
  let s = url.trim();
  s = s.replace(/^[a-z]+:\/\//i, ""); // https://
  s = s.replace(/^([^@]+@)?/, ""); // user@ (scp-style / userinfo)
  s = s.replace(/\.git$/i, "");
  return s;
}

const generalValue = computed(() => {
  const loc = appConfig.value?.locale;
  if (loc === "en") return t("settings.language.english");
  if (loc === "zh-CN") return t("settings.language.chinese");
  return t("settings.language.system");
});
const lockingValue = computed(() => lockModeLabel(appConfig.value?.lock_mode));
const identityValue = computed(() => {
  if (
    identityType.value === "ssh_ed25519" ||
    identityType.value === "ssh_rsa"
  ) {
    return t("settings.hub.identitySsh");
  }
  return identityEncrypted.value
    ? t("settings.hub.identityEncrypted")
    : t("settings.hub.identityPlaintext");
});
const repositoryValue = computed(() =>
  repoConfig.value ? repoHost(repoConfig.value.url) : "",
);

async function loadSummaries() {
  loading.value = true;
  error.value = "";
  // Each summary source resolves independently. A single failure (a corrupted
  // repo.json failing its AEAD tag on Android, a transient IPC blip) must not
  // hide the whole menu — this page's only job is navigation, so the rows stay
  // tappable and a drill-in surfaces the sub-page's own error. Settle each on
  // its own; the value computeds degrade to defaults for any that didn't load.
  const [app, repo, auth] = await Promise.allSettled([
    getAppConfig(),
    getConfig(),
    getAuthState(),
  ]);
  if (app.status === "fulfilled") appConfig.value = app.value;
  if (repo.status === "fulfilled") repoConfig.value = repo.value;
  if (auth.status === "fulfilled") {
    identityEncrypted.value = auth.value.encrypted;
    identityType.value = auth.value.identity_type;
  }
  // Only banner when nothing resolved at all — a partial load still navigates.
  if (
    app.status === "rejected" &&
    repo.status === "rejected" &&
    auth.status === "rejected"
  ) {
    const appError = (app as PromiseRejectedResult).reason as AppError;
    error.value = appError?.message || t("settings.commit.loadFailed");
  }
  loading.value = false;
}

onMounted(() => {
  loadSummaries();
});
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <BaseHeader
      :back-fallback="{ name: 'entries' }"
      :title="t('settings.title')"
      :title-icon="Settings"
    />

    <div v-if="loading" class="text-center text-muted py-8">
      {{ t("common.loading") }}
    </div>

    <template v-else>
      <BaseAlert v-if="error" variant="danger" class="mb-4">
        {{ error }}
      </BaseAlert>

      <BaseCard as="section" class="hub">
        <div
          class="hub-row"
          tabindex="0"
          role="button"
          :aria-label="`${t('settings.hub.general')} — ${generalValue}`"
          @click="router.push({ name: 'settingsGeneral' })"
          @keydown.enter="router.push({ name: 'settingsGeneral' })"
          @keydown.space.prevent="router.push({ name: 'settingsGeneral' })"
        >
          <BaseIcon :icon="SlidersHorizontal" :size="20" class="text-muted" />
          <span class="hub-title">{{ t("settings.hub.general") }}</span>
          <span class="hub-value">{{ generalValue }}</span>
          <BaseIcon :icon="ChevronRight" :size="20" class="text-muted" />
        </div>

        <div
          class="hub-row"
          tabindex="0"
          role="button"
          :aria-label="`${t('settings.hub.locking')} — ${lockingValue}`"
          @click="router.push({ name: 'settingsLocking' })"
          @keydown.enter="router.push({ name: 'settingsLocking' })"
          @keydown.space.prevent="router.push({ name: 'settingsLocking' })"
        >
          <BaseIcon :icon="Lock" :size="20" class="text-muted" />
          <span class="hub-title">{{ t("settings.hub.locking") }}</span>
          <span class="hub-value">{{ lockingValue }}</span>
          <BaseIcon :icon="ChevronRight" :size="20" class="text-muted" />
        </div>

        <div
          class="hub-row"
          tabindex="0"
          role="button"
          :aria-label="`${t('settings.hub.identity')} — ${identityValue}`"
          @click="router.push({ name: 'settingsIdentity' })"
          @keydown.enter="router.push({ name: 'settingsIdentity' })"
          @keydown.space.prevent="router.push({ name: 'settingsIdentity' })"
        >
          <BaseIcon :icon="KeyRound" :size="20" class="text-muted" />
          <span class="hub-title">{{ t("settings.hub.identity") }}</span>
          <span class="hub-value">{{ identityValue }}</span>
          <BaseIcon :icon="ChevronRight" :size="20" class="text-muted" />
        </div>

        <div
          class="hub-row"
          tabindex="0"
          role="button"
          :aria-label="`${t('settings.hub.repository')} — ${repositoryValue}`"
          @click="router.push({ name: 'settingsRepository' })"
          @keydown.enter="router.push({ name: 'settingsRepository' })"
          @keydown.space.prevent="router.push({ name: 'settingsRepository' })"
        >
          <BaseIcon :icon="Database" :size="20" class="text-muted" />
          <span class="hub-title">{{ t("settings.hub.repository") }}</span>
          <span class="hub-value">{{ repositoryValue }}</span>
          <BaseIcon :icon="ChevronRight" :size="20" class="text-muted" />
        </div>
      </BaseCard>
    </template>
  </main>
</template>

<style scoped>
.hub {
  padding: 0.25rem 1rem;
}
.hub-row {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  min-height: 3rem; /* 48px touch target */
  padding: 0.6rem 0;
  cursor: pointer;
  border-radius: var(--radius-sm);
  transition: background-color 0.15s;
}
.hub-row + .hub-row {
  border-top: 1px solid var(--color-edge);
}
.hub-row:hover,
.hub-row:focus-visible {
  background: var(--color-hover, var(--color-edge));
  outline: none;
}
.hub-title {
  font-size: 0.95rem;
}
.hub-value {
  margin-left: auto;
  font-size: 0.8rem;
  color: var(--color-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 50%;
}
</style>
