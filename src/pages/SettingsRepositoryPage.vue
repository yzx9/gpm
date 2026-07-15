<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import type {
  AppError,
  AuthenticityConfig,
  CommitIdentity,
  RepoConfig,
  VerifyMode,
} from "@/api";
import {
  getAuthenticityConfig,
  getCommitIdentityDefault,
  getConfig,
  getGpgKeyParseWarnings,
  importTrustedGpgKeyFile,
  removeTrustedGpgKey,
  removeTrustedKey,
  setCommitIdentity,
  setVerificationMode,
  trustHeadSigner,
} from "@/api";
import BaseAlert from "@/components/base/BaseAlert.vue";
import BaseButton from "@/components/base/BaseButton.vue";
import BaseCard from "@/components/base/BaseCard.vue";
import BaseHeader from "@/components/base/BaseHeader.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseInput from "@/components/base/BaseInput.vue";
import BaseModalShell from "@/components/base/BaseModalShell.vue";
import BaseSegmentedControl from "@/components/base/BaseSegmentedControl.vue";
import { useToast } from "@/composables";
import { Database, FileUp, History, KeyRound, Plus } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { onBeforeRouteLeave, useRouter } from "vue-router";

const router = useRouter();
const { toast } = useToast();
const { t } = useI18n();

const config = ref<RepoConfig | null>(null);
const loading = ref(false);
const error = ref("");

const isSsh = ref(false);

// ── Commit identity state ────────────────────────────────────────────────
const commitName = ref("");
const commitEmail = ref("");
const commitLoading = ref(false);
const commitDefault = ref<CommitIdentity | null>(null);

async function loadConfig() {
  loading.value = true;
  error.value = "";
  try {
    config.value = await getConfig();
    isSsh.value = config.value.ssh_key !== null;
    commitName.value = config.value.commit_user_name ?? "";
    commitEmail.value = config.value.commit_user_email ?? "";
    // The default-identity hint is a nicety (the form works without it); a
    // failure here (e.g. no git user.name configured) must not raise a
    // page-level "load failed" banner over the usable cards. Leave it null and
    // let the hint ternary degrade to empty.
    commitDefault.value = await getCommitIdentityDefault().catch(() => null);
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("settings.commit.loadFailed");
  } finally {
    loading.value = false;
  }
}

async function onSaveCommitIdentity(): Promise<boolean> {
  error.value = "";
  commitLoading.value = true;
  try {
    const updated = await setCommitIdentity(
      commitName.value.trim() || null,
      commitEmail.value.trim() || null,
    );
    config.value = updated;
    // Re-sync from the response (trimmed) so a successful Save clears dirty.
    commitName.value = updated.commit_user_name ?? "";
    commitEmail.value = updated.commit_user_email ?? "";
    toast.success(t("settings.commit.saved"));
    return true;
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("settings.commit.saveFailed");
    return false;
  } finally {
    commitLoading.value = false;
  }
}

// ── Repository authenticity state ────────────────────────────────────────
const authConfig = ref<AuthenticityConfig | null>(null);
const authLoading = ref(false);
/** Per-key parse warnings for the persisted trusted GPG keys — non-empty when
 * a previously-trusted key later fails to re-parse (surfaces the silent
 * Verified→UnverifiedSignature downgrade so the user can re-add or remove). */
const gpgWarnings = ref<string[]>([]);

/** SSH + GPG trusted keys flattened into render rows tagged by origin list, so
 * the combined list can badge GPG entries and route each Remove to the right
 * command without sniffing the fingerprint string. */
const trustedKeyRows = computed<
  ReadonlyArray<{ kind: "ssh" | "gpg"; fingerprint: string; label: string }>
>(() => {
  const cfg = authConfig.value;
  if (!cfg) return [];
  return [
    ...cfg.trusted_keys.map((k) => ({
      kind: "ssh" as const,
      fingerprint: k.fingerprint,
      label: k.label,
    })),
    ...cfg.trusted_gpg_keys.map((k) => ({
      kind: "gpg" as const,
      fingerprint: k.fingerprint,
      label: k.label,
    })),
  ];
});

// Verification-mode pills (labels capitalize via CSS to match the prior look).
const VERIFY_MODES: {
  label: VerifyMode;
  value: VerifyMode;
  labelClass: string;
}[] = (["off", "audit", "enforce"] as VerifyMode[]).map((m) => ({
  label: m,
  value: m,
  labelClass: "capitalize",
}));

async function loadAuthConfig() {
  try {
    // Warnings are a Settings-only concern (separate command, NOT on the
    // entry-list badge path); fetch alongside the config. A warnings failure
    // must not brick the whole card.
    const [cfg, warnings] = await Promise.all([
      getAuthenticityConfig(),
      getGpgKeyParseWarnings().catch(() => [] as string[]),
    ]);
    authConfig.value = cfg;
    gpgWarnings.value = warnings;
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("settings.auth.loadConfigFailed");
  }
}

async function onModeChange(mode: VerifyMode) {
  if (!authConfig.value) return;
  authLoading.value = true;
  error.value = "";
  try {
    const effective = await setVerificationMode(mode);
    authConfig.value.mode = effective;
  } catch (e) {
    const appError = e as AppError;
    if (mode === "enforce") {
      error.value = t("settings.auth.enforceNeedsKey");
      // Revert the radio to the current effective mode.
      authConfig.value.mode = authConfig.value.mode;
    } else {
      error.value = appError?.message || t("settings.auth.setModeFailed");
    }
  } finally {
    authLoading.value = false;
  }
}

async function onRemoveKey(fingerprint: string, kind: "ssh" | "gpg") {
  if (!confirm(t("settings.auth.removeConfirm"))) return;
  authLoading.value = true;
  try {
    if (kind === "gpg") {
      await removeTrustedGpgKey(fingerprint);
    } else {
      await removeTrustedKey(fingerprint);
    }
    toast.success(t("settings.auth.removedToast"));
    await loadAuthConfig();
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("settings.auth.removeFailed");
  } finally {
    authLoading.value = false;
  }
}

async function onImportGpgKey() {
  // Prompt for a label up front; the backend pre-fills from the filename when
  // this is blank, so empty is fine too.
  const label = window.prompt(t("settings.auth.importGpgPrompt"), "");
  if (label === null) return;
  authLoading.value = true;
  error.value = "";
  try {
    await importTrustedGpgKeyFile(label.trim());
    toast.success(t("settings.auth.importGpgToast"));
    await loadAuthConfig();
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("settings.auth.importGpgFailed");
  } finally {
    authLoading.value = false;
  }
}

async function onTrustHead() {
  const label = window.prompt(t("settings.auth.trustHeadPrompt"), "signer");
  if (label === null) return;
  authLoading.value = true;
  try {
    await trustHeadSigner(label.trim() || "signer");
    toast.success(t("settings.auth.trustHeadToast"));
    await loadAuthConfig();
  } catch (e) {
    const appError = e as AppError;
    error.value = appError?.message || t("settings.auth.trustHeadFailed");
  } finally {
    authLoading.value = false;
  }
}

function openHistory() {
  router.push({ name: "history" });
}

// ── Deferred-save dirty tracking + leave guard (Commit Identity) ──────────
// The commit-identity form stages edits in refs and only commits on its own
// Save button. Leaving the page with uncommitted edits prompts Discard / Save /
// Keep editing, so a stray back-tap never silently throws away typed input.
const commitDirty = computed(() => {
  const name = config.value?.commit_user_name ?? "";
  const email = config.value?.commit_user_email ?? "";
  return commitName.value.trim() !== name || commitEmail.value.trim() !== email;
});
const hasUnsavedChanges = computed(() => commitDirty.value);

// Commit the dirty commit-identity form. Returns false on failure so the leave
// guard can keep the user on the page to see the error.
async function commitAllPending(): Promise<boolean> {
  if (!commitDirty.value) return true;
  return onSaveCommitIdentity();
}

type UnsavedChoice = "discard" | "save" | "cancel";
const unsavedOpen = ref(false);
let pendingResolve: ((c: UnsavedChoice) => void) | null = null;
// Re-entrancy guard: if a second navigation supersedes the guarded one while
// the modal is open, only the latest token's resolver runs; older ones no-op.
let promptToken = 0;

function promptUnsaved(): Promise<UnsavedChoice> {
  const token = ++promptToken;
  unsavedOpen.value = true;
  return new Promise<UnsavedChoice>((resolve) => {
    pendingResolve = (c) => {
      if (token === promptToken) resolve(c);
    };
  });
}

function resolveUnsaved(c: UnsavedChoice) {
  const r = pendingResolve;
  pendingResolve = null;
  unsavedOpen.value = false;
  r?.(c);
}

onBeforeRouteLeave(async () => {
  if (!hasUnsavedChanges.value) return true;
  const choice = await promptUnsaved();
  if (choice === "cancel") return false;
  if (choice === "save") {
    const ok = await commitAllPending();
    if (!ok) return false; // keep the user on the page so the error is visible
  }
  return true; // discard, or save succeeded
});

onMounted(() => {
  loadConfig();
  loadAuthConfig();
});
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <BaseHeader
      :back-fallback="{ name: 'settings' }"
      :title="t('settings.hub.repository')"
      :title-icon="Database"
    />

    <div v-if="loading" class="text-center text-muted py-8">
      {{ t("common.loading") }}
    </div>

    <template v-else>
      <!-- One alert slot covers both a load failure (config null, no cards)
           and an operation failure (config loaded, form stays usable). -->
      <BaseAlert v-if="error" variant="danger" class="mb-4">
        {{ error }}
      </BaseAlert>

      <div v-if="config" class="flex flex-col gap-4">
        <!-- Repo info -->
        <BaseCard as="section">
          <h2 class="text-sm font-medium mb-2">
            {{ t("settings.repo.title") }}
          </h2>
          <div class="text-sm text-muted break-all">{{ config.url }}</div>
          <div class="text-xs text-muted mt-1">
            {{
              isSsh
                ? t("settings.repo.auth.ssh")
                : config.pat
                  ? t("settings.repo.auth.pat")
                  : t("settings.repo.auth.none")
            }}
          </div>
        </BaseCard>

        <!-- Commit identity -->
        <BaseCard as="section" :border="commitDirty ? 'accent' : 'edge'">
          <h2 class="text-sm font-medium mb-2">
            {{ t("settings.commit.title") }}
            <span
              v-if="commitDirty"
              class="ml-1 text-xs font-normal"
              style="color: var(--color-accent)"
              >{{ t("settings.commit.unsaved") }}</span
            >
          </h2>
          <p class="text-xs text-muted mb-3">
            {{
              t("settings.commit.hint", {
                default: commitDefault
                  ? t("settings.commit.default", {
                      name: commitDefault.name,
                      email: commitDefault.email,
                    })
                  : "",
              })
            }}
          </p>
          <div class="flex flex-col gap-1 mb-3">
            <label for="commit-name" class="text-xs text-muted">{{
              t("settings.commit.nameLabel")
            }}</label>
            <BaseInput
              id="commit-name"
              v-model="commitName"
              type="text"
              :placeholder="t('settings.commit.namePlaceholder')"
              autocomplete="off"
              :disabled="commitLoading"
            />
          </div>
          <div class="flex flex-col gap-1 mb-3">
            <label for="commit-email" class="text-xs text-muted">{{
              t("settings.commit.emailLabel")
            }}</label>
            <BaseInput
              id="commit-email"
              v-model="commitEmail"
              type="email"
              :placeholder="t('settings.commit.emailPlaceholder')"
              autocomplete="off"
              :disabled="commitLoading"
            />
          </div>
          <BaseButton
            variant="action"
            :loading="commitLoading"
            @click="onSaveCommitIdentity"
          >
            {{ t("settings.commit.save") }}
          </BaseButton>
        </BaseCard>

        <!-- SSH key management — the key view/export lives on its own route so
           Android back returns here instead of the settings hub. -->
        <BaseCard as="section" v-if="isSsh">
          <h2 class="text-sm font-medium mb-3">
            {{ t("settings.ssh.title") }}
          </h2>
          <BaseButton variant="action" @click="router.push({ name: 'sshKey' })">
            <BaseIcon :icon="KeyRound" /> {{ t("settings.ssh.manage") }}
          </BaseButton>
        </BaseCard>

        <!-- Repository authenticity -->
        <BaseCard as="section" v-if="authConfig">
          <h2 class="text-sm font-medium mb-3">
            {{ t("settings.auth.title") }}
          </h2>
          <p class="text-xs text-muted mb-3">
            {{ t("settings.auth.description") }}
          </p>

          <!-- Mode selector -->
          <BaseSegmentedControl
            class="mb-3"
            name="verify-mode"
            :legend="t('settings.auth.legend')"
            :model-value="authConfig.mode"
            :options="VERIFY_MODES"
            :disabled="authLoading"
            @change="onModeChange"
          >
            <template #hint>
              <p class="text-xs text-muted mt-1">
                <template v-if="authConfig.mode === 'off'">{{
                  t("settings.auth.offHint")
                }}</template>
                <template v-else-if="authConfig.mode === 'audit'">{{
                  t("settings.auth.auditHint")
                }}</template>
                <template v-else>{{ t("settings.auth.enforceHint") }}</template>
              </p>
            </template>
          </BaseSegmentedControl>

          <!-- Trusted signing keys (SSH + GPG combined; GPG entries tagged) -->
          <div class="text-xs text-muted mb-1">
            {{
              t("settings.auth.trustedKeys", { count: trustedKeyRows.length })
            }}
          </div>
          <ul v-if="trustedKeyRows.length" class="flex flex-col gap-1 mb-2">
            <li
              v-for="row in trustedKeyRows"
              :key="row.kind + ':' + row.fingerprint"
              class="key-row"
            >
              <code class="text-xs break-all flex-1">{{
                row.fingerprint
              }}</code>
              <span
                v-if="row.kind === 'gpg'"
                class="text-[0.6rem] text-default px-1 rounded-sm bg-edge shrink-0"
                >GPG</span
              >
              <span class="text-xs text-muted mx-2 truncate">{{
                row.label
              }}</span>
              <button
                type="button"
                class="btn-copy"
                @click="onRemoveKey(row.fingerprint, row.kind)"
              >
                {{ t("settings.auth.remove") }}
              </button>
            </li>
          </ul>
          <p v-else class="text-xs text-muted mb-2">
            {{ t("settings.auth.noTrustedKeys") }}
          </p>
          <p
            v-if="gpgWarnings.length"
            class="text-xs text-warning mb-2 break-words"
          >
            {{ t("settings.auth.gpgWarning", { count: gpgWarnings.length }) }}
          </p>

          <div class="flex flex-col gap-2">
            <BaseButton
              v-if="trustedKeyRows.length === 0"
              variant="action"
              @click="onTrustHead"
            >
              <BaseIcon :icon="KeyRound" /> {{ t("settings.auth.trustHead") }}
            </BaseButton>
            <BaseButton
              variant="action"
              @click="router.push({ name: 'addKey' })"
            >
              <BaseIcon :icon="Plus" /> {{ t("settings.auth.addKey") }}
            </BaseButton>
            <BaseButton
              variant="action"
              :disabled="authLoading"
              @click="onImportGpgKey"
            >
              <BaseIcon :icon="FileUp" /> {{ t("settings.auth.importGpg") }}
            </BaseButton>
            <BaseButton variant="action" @click="openHistory">
              <BaseIcon :icon="History" /> {{ t("settings.auth.viewHistory") }}
            </BaseButton>
          </div>
        </BaseCard>
      </div>
    </template>

    <!-- Unsaved-changes leave guard. z=50 layering; backdrop or Android-back
         = Keep editing. -->
    <BaseModalShell
      v-if="unsavedOpen"
      variant="sheet"
      :z="50"
      role="alertdialog"
      :aria-label="t('settings.unsaved.ariaLabel')"
      @close="resolveUnsaved('cancel')"
    >
      <h2 class="text-lg font-medium mb-2">
        {{ t("settings.unsaved.title") }}
      </h2>
      <p class="text-sm text-muted mb-4">{{ t("settings.unsaved.body") }}</p>
      <div class="flex flex-col gap-2">
        <BaseButton variant="action" @click="resolveUnsaved('save')">{{
          t("settings.unsaved.save")
        }}</BaseButton>
        <BaseButton variant="secondary" @click="resolveUnsaved('cancel')">{{
          t("settings.unsaved.keep")
        }}</BaseButton>
        <BaseButton
          variant="action-danger"
          @click="resolveUnsaved('discard')"
          >{{ t("settings.unsaved.discard") }}</BaseButton
        >
      </div>
    </BaseModalShell>
  </main>
</template>

<style scoped>
.key-row {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.4rem 0.5rem;
  border: 1px solid var(--color-edge);
  border-radius: var(--radius-sm);
  background: var(--color-input);
}
</style>
