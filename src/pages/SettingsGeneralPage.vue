<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import type { AppConfig, AppError, SecureScreenMode } from "@/api";
import {
  resetConfig as apiResetConfig,
  getAppConfig,
  resolvedLocale,
  setAutosync,
  setLocalePref,
  setThemeMode,
} from "@/api";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseCard from "@/components/base/BaseCard.vue";
import BaseHeader from "@/components/base/BaseHeader.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import BaseModalShell from "@/components/base/BaseModalShell.vue";
import BaseSegmentedControl from "@/components/base/BaseSegmentedControl.vue";
import { useSecureScreen, useToast } from "@/composables";
import { normalizeSupported, setLocale } from "@/i18n";
import { applyTheme, normalizeThemeMode, type ThemeMode } from "@/theme";
import { SlidersHorizontal, Trash2 } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";

const router = useRouter();
const { toast } = useToast();
const { t } = useI18n();

// Display-language preference: "system" (track the device locale) or a pinned
// locale. Loaded from app.json on mount; the picker applies it live.
const localeSelection = ref<"system" | "en" | "zh-CN">("system");

async function loadLocalePref(): Promise<void> {
  try {
    const app = await getAppConfig();
    localeSelection.value =
      app.locale === "en" || app.locale === "zh-CN" ? app.locale : "system";
  } catch {
    // No app config yet — leave "system".
  }
}

async function onLocaleChange(selection: string): Promise<void> {
  const prev = localeSelection.value;
  try {
    // Apply the locale in-memory first and persist only on success, so a
    // failure can't leave app.json pinned to a locale the picker reverted to.
    if (selection === "system") {
      // "Track system" resolves through the backend, which normalizes the
      // device locale — apply that immediately so the switch is visible.
      await setLocale(normalizeSupported(await resolvedLocale()));
      await setLocalePref(null);
    } else if (selection === "en" || selection === "zh-CN") {
      await setLocale(selection);
      await setLocalePref(selection);
    } else {
      return; // unknown option — ignore
    }
    localeSelection.value = selection as "system" | "en" | "zh-CN";
    toast.success(t("settings.language.applied"));
  } catch {
    localeSelection.value = prev; // roll back the picker on failure
    toast.danger(t("settings.language.failed"));
  }
}

// Color-scheme preference: "system" (track the OS via the CSS media query) or a
// pinned light/dark. Loaded from app.json on mount; the picker applies it live.
const themeSelection = ref<ThemeMode>("system");
// Guards the picker against rapid taps firing concurrent set_theme_mode calls —
// without it, the last IPC to resolve wins regardless of tap order (the same
// get→mutate→save race autosyncLoading guards on the AutoSync toggle).
const themeLoading = ref(false);

async function onThemeChange(selection: string): Promise<void> {
  if (themeLoading.value) return;
  const prev = themeSelection.value;
  const mode = normalizeThemeMode(selection);
  // Apply in-memory first and persist only on success, mirroring onLocaleChange.
  applyTheme(mode);
  themeSelection.value = mode;
  themeLoading.value = true;
  try {
    await setThemeMode(mode === "system" ? null : mode);
    toast.success(t("settings.theme.applied"));
  } catch {
    themeSelection.value = prev; // roll back the picker + the applied theme
    applyTheme(prev);
    toast.danger(t("settings.theme.failed"));
  } finally {
    themeLoading.value = false;
  }
}

const appConfig = ref<AppConfig | null>(null);
const loading = ref(false);
const error = ref("");

const { secureScreenMode, secureAvailable, setSecureScreenMode } =
  useSecureScreen();

async function loadConfig() {
  loading.value = true;
  error.value = "";
  try {
    appConfig.value = await getAppConfig();
    themeSelection.value = normalizeThemeMode(appConfig.value.theme_mode);
    await loadLocalePref();
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("settings.commit.loadFailed");
  } finally {
    loading.value = false;
  }
}

// Guards the picker against rapid taps firing concurrent setSecureScreenMode
// calls — mirrors the theme picker's loading guard.
const secureScreenLoading = ref(false);

async function onSecureScreenChange(selection: string) {
  if (secureScreenLoading.value) return;
  // The picker emits the option value; narrow to the union. An unexpected value
  // (defensive) resolves to "sensitive".
  const mode: SecureScreenMode =
    selection === "off" || selection === "always" ? selection : "sensitive";
  secureScreenLoading.value = true;
  try {
    const ok = await setSecureScreenMode(mode);
    if (!ok) {
      toast.danger(t("settings.secureScreen.saveFailed"));
      return;
    }
    toast.success(t(`settings.secureScreen.${mode}Toast`));
  } finally {
    secureScreenLoading.value = false;
  }
}

const autosyncEnabled = computed(() => appConfig.value?.autosync ?? true);
// Guards the toggle against rapid taps firing concurrent setAutosync calls —
// without it, the last IPC to resolve wins appConfig regardless of tap order.
const autosyncLoading = ref(false);

async function onAutosyncChange(enabled: boolean) {
  if (!appConfig.value) return;
  autosyncLoading.value = true;
  try {
    appConfig.value = await setAutosync(enabled);
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("settings.autosync.setFailed");
  } finally {
    autosyncLoading.value = false;
  }
}

// Reset is gated behind a type-"RESET"-to-confirm modal: a stray tap can't
// trigger this unrecoverable wipe, and no passphrase is required, so a user
// who forgot theirs can still reset.
const RESET_CONFIRM_WORD = "RESET";
const resetOpen = ref(false);
const resetConfirmText = ref("");
const resetReady = computed(
  () => resetConfirmText.value.trim().toUpperCase() === RESET_CONFIRM_WORD,
);

function resetConfig() {
  resetConfirmText.value = "";
  resetOpen.value = true;
}

async function doReset() {
  if (!resetReady.value) return;
  try {
    await apiResetConfig();
    resetOpen.value = false;
    const failure = await router.replace({ name: "setup" });
    // vue-router resolves a cancelled/aborted nav as a NavigationFailure, not a
    // throw. The backend is already wiped — force a re-init so the app re-enters at
    // /setup instead of stranding the user on stale Settings.
    if (failure) window.location.reload();
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("settings.reset.failed");
    resetOpen.value = false;
  }
}

onMounted(() => {
  loadConfig();
});
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <BaseHeader
      :back-fallback="{ name: 'settings' }"
      :title="t('settings.hub.general')"
      :title-icon="SlidersHorizontal"
    />

    <div v-if="loading" class="text-center text-muted py-8">
      {{ t("common.loading") }}
    </div>

    <BaseAlert v-else-if="error" variant="danger" class="mb-4">
      {{ error }}
    </BaseAlert>

    <div v-else-if="appConfig" class="flex flex-col gap-4">
      <!-- Display language -->
      <BaseCard as="section">
        <h2 class="text-sm font-medium mb-2">
          {{ t("settings.language.title") }}
        </h2>
        <p class="text-xs text-muted mb-3">
          {{ t("settings.language.description") }}
        </p>
        <BaseSegmentedControl
          name="display-language"
          :legend="t('settings.language.legend')"
          :model-value="localeSelection"
          :options="[
            { label: t('settings.language.system'), value: 'system' },
            { label: t('settings.language.english'), value: 'en' },
            { label: t('settings.language.chinese'), value: 'zh-CN' },
          ]"
          @change="onLocaleChange"
        />
      </BaseCard>

      <!-- Theme (System / Light / Dark) — System tracks the OS via the CSS
           media query; Light/Dark pin via <html data-theme>. -->
      <BaseCard as="section">
        <h2 class="text-sm font-medium mb-2">
          {{ t("settings.theme.title") }}
        </h2>
        <p class="text-xs text-muted mb-3">
          {{ t("settings.theme.description") }}
        </p>
        <BaseSegmentedControl
          name="theme-mode"
          :legend="t('settings.theme.legend')"
          :model-value="themeSelection"
          :options="[
            { label: t('settings.theme.system'), value: 'system' },
            { label: t('settings.theme.light'), value: 'light' },
            { label: t('settings.theme.dark'), value: 'dark' },
          ]"
          :disabled="themeLoading"
          @change="onThemeChange"
        />
      </BaseCard>

      <!-- Screen capture protection (Android FLAG_SECURE) — Android only -->
      <BaseCard as="section" v-if="secureAvailable">
        <h2 class="text-sm font-medium mb-2">
          {{ t("settings.secureScreen.title") }}
        </h2>
        <p class="text-xs text-muted mb-3">
          {{ t("settings.secureScreen.description") }}
        </p>
        <BaseSegmentedControl
          name="secure-screen"
          :legend="t('settings.secureScreen.legend')"
          :model-value="secureScreenMode"
          :options="[
            { label: t('settings.secureScreen.off'), value: 'off' },
            {
              label: t('settings.secureScreen.sensitive'),
              value: 'sensitive',
            },
            { label: t('settings.secureScreen.always'), value: 'always' },
          ]"
          :disabled="secureScreenLoading"
          @change="onSecureScreenChange"
        >
          <template #hint>
            <p class="text-xs text-muted mt-1">
              {{ t(`settings.secureScreen.${secureScreenMode}Hint`) }}
            </p>
          </template>
        </BaseSegmentedControl>
      </BaseCard>

      <!-- AutoSync -->
      <BaseCard as="section">
        <h2 class="text-sm font-medium mb-3">
          {{ t("settings.autosync.title") }}
        </h2>
        <BaseSegmentedControl
          class="mb-3"
          name="autosync"
          :legend="t('settings.autosync.legend')"
          :model-value="autosyncEnabled"
          :options="[
            { label: t('settings.autosync.on'), value: true },
            { label: t('settings.autosync.off'), value: false },
          ]"
          :disabled="autosyncLoading"
          @change="onAutosyncChange"
        >
          <template #hint>
            <p class="text-xs text-muted mt-1">
              <template v-if="autosyncEnabled">{{
                t("settings.autosync.onHint")
              }}</template>
              <template v-else>{{ t("settings.autosync.offHint") }}</template>
            </p>
          </template>
        </BaseSegmentedControl>
      </BaseCard>

      <!-- Danger zone -->
      <BaseCard as="section" border="danger">
        <h2 class="text-sm font-medium mb-2 text-danger">
          {{ t("settings.reset.title") }}
        </h2>
        <BaseButton variant="action-danger" @click="resetConfig">
          <BaseIcon :icon="Trash2" /> {{ t("settings.reset.button") }}
        </BaseButton>
        <p class="text-xs text-muted mt-1">
          {{ t("settings.reset.description") }}
        </p>
      </BaseCard>
    </div>

    <!-- Reset confirmation: type RESET to confirm. -->
    <BaseModalShell
      v-if="resetOpen"
      variant="center"
      :z="80"
      role="alertdialog"
      :aria-label="t('settings.reset.ariaLabel')"
      @close="resetOpen = false"
    >
      <h2 class="text-lg font-medium text-danger mb-3">
        {{ t("settings.reset.modalTitle") }}
      </h2>
      <BaseAlert variant="danger" class="mb-4">
        {{ t("settings.reset.modalBody") }}
      </BaseAlert>
      <div class="flex flex-col gap-1 mb-4">
        <label class="text-sm font-medium" for="reset-confirm">{{
          t("settings.reset.typeReset")
        }}</label>
        <BaseInput
          id="reset-confirm"
          v-model="resetConfirmText"
          autocomplete="off"
          autofocus
        />
      </div>
      <div class="flex gap-2 justify-end">
        <BaseButton variant="secondary" @click="resetOpen = false">{{
          t("common.button.cancel")
        }}</BaseButton>
        <BaseButton variant="danger" :disabled="!resetReady" @click="doReset">
          <BaseIcon :icon="Trash2" /> {{ t("settings.reset.confirm") }}
        </BaseButton>
      </div>
    </BaseModalShell>
  </main>
</template>
