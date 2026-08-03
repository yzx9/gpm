<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
import {
  areClipboardNotificationsEnabled,
  isBiometricAvailable,
  openClipboardNotificationSettings,
  openSecuritySettings,
  type BiometricState,
} from "@/api";
import BaseCard from "@/components/base/BaseCard.vue";
import BaseHeader from "@/components/base/BaseHeader.vue";
import BaseIcon from "@/components/base/BaseIcon.vue";
import BaseSpinner from "@/components/base/BaseSpinner.vue";
import { useSecureScreen, useToast } from "@/composables";
import {
  Bell,
  ChevronRight,
  Clipboard,
  FileText,
  Fingerprint,
  Globe,
} from "@lucide/vue";
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useI18n } from "vue-i18n";

// A permissions/data-access transparency page. Each row names a surface gpm
// touches, says why, and — only where Android lets the user change it and may
// have suppressed the prompt (notifications, biometric) — offers a whole-row
// tap that deep-links into the system settings. Non-configurable surfaces
// (clipboard, network, files) are plain explainers. Carries no secret, so the
// route is NOT FLAG_SECURE (capturable, like Security). One card per row;
// the colored status word doubles as the tappable action; 48px whole-row tap.

const { t } = useI18n();
const { toast } = useToast();
const { secureAvailable } = useSecureScreen();

// Tri-state probe results: the resolved state, or "unknown" while loading / when
// the probe failed (the row then shows a spinner, not a misleading status).
const notificationsState = ref<"granted" | "blocked" | "unknown">("unknown");
const biometricState = ref<BiometricState | "unknown">("unknown");

// Monotonic generation tag: a probe started after a rapid mount/unmount/remount
// (or two resume signals firing close together) must be able to discard a slower
// earlier result so stale state can't overwrite fresh state. Mirrors the
// AppState generation-tag pattern (lib.rs lock_generation / clipboard_clear).
let probeGen = 0;

async function probe() {
  const gen = ++probeGen;
  const [n, b] = await Promise.allSettled([
    areClipboardNotificationsEnabled(),
    isBiometricAvailable(),
  ]);
  if (gen !== probeGen) return; // a newer probe started; this result is stale
  // A rejected notif probe degrades to "blocked" (tappable recovery) rather than
  // an infinite spinner — this page's whole purpose is the recovery lever, so a
  // plugin flake must not strand the row. (isBiometricAvailable swallows its own
  // errors → "unavailable", so it never rejects here.)
  notificationsState.value =
    n.status === "fulfilled" && n.value ? "granted" : "blocked";
  biometricState.value = b.status === "fulfilled" ? b.value : "unknown";
}

// Resume refresh: `visibilitychange` is the reliable Android app-resume signal
// (it fires when the user returns from the system-settings screen the deep-link
// opened). `window` focus is a belt-and-suspenders second signal (the codebase
// already double-covers this OEM edge in useAppLockState); and navigating away
// from the page and back re-mounts it, re-running onMounted. Named handler so
// removeEventListener removes the SAME reference (no anonymous-arrow leak).
const onResume = () => {
  void probe();
};

onMounted(() => {
  void probe();
  document.addEventListener("visibilitychange", onResume);
  window.addEventListener("focus", onResume);
});
onUnmounted(() => {
  document.removeEventListener("visibilitychange", onResume);
  window.removeEventListener("focus", onResume);
});

// The deep-link returns whether a handler activity was found; `false` or a throw
// (exotic OEM ROM lacking the target) is surfaced as a toast rather than a
// silent dead tap — the whole point of this page is a visible recovery lever.
async function openNotifications() {
  try {
    if (!(await openClipboardNotificationSettings())) {
      toast.danger(t("permissions.notifications.failed"));
    }
  } catch {
    toast.danger(t("permissions.notifications.failed"));
  }
}
async function openBiometric() {
  try {
    if (!(await openSecuritySettings())) {
      toast.danger(t("permissions.biometric.failed"));
    }
  } catch {
    toast.danger(t("permissions.biometric.failed"));
  }
}

// Notification row is actionable only when blocked.
const notifTap = computed(() => notificationsState.value === "blocked");
const notifStatus = computed(() => {
  switch (notificationsState.value) {
    case "granted":
      return { text: t("permissions.notifications.granted"), tone: "muted" };
    case "blocked":
      return { text: t("permissions.notifications.blocked"), tone: "danger" };
    default:
      return null; // unknown → spinner
  }
});
const notifAria = computed(() =>
  notifStatus.value
    ? `${t("permissions.notifications.title")} — ${notifStatus.value.text}`
    : t("permissions.notifications.title"),
);

// Biometric row is actionable whenever enrollment could help: nothing enrolled
// (no_enrollment), or only a weak Class-2 print (weak_enrolled). On
// Class-3-capable hardware enrolling a Class-3 fingerprint flips this to
// `available`; on Class-2-only hardware the deep-link is a harmless no-op
// round-trip — and mapBiometricState can't tell the two apart, so offer it.
const bioTap = computed(
  () =>
    biometricState.value === "no_enrollment" ||
    biometricState.value === "weak_enrolled",
);
const bioStatus = computed(() => {
  switch (biometricState.value) {
    case "available":
      return { text: t("permissions.biometric.available"), tone: "muted" };
    case "no_enrollment":
      return { text: t("permissions.biometric.noEnrollment"), tone: "accent" };
    case "weak_enrolled":
      return { text: t("permissions.biometric.weakEnrolled"), tone: "accent" };
    case "unavailable":
      return { text: t("permissions.biometric.unavailable"), tone: "muted" };
    default:
      return null; // unknown → spinner
  }
});
const bioAria = computed(() =>
  bioStatus.value
    ? `${t("permissions.biometric.title")} — ${bioStatus.value.text}`
    : t("permissions.biometric.title"),
);

function toneClass(tone: string) {
  if (tone === "danger") return "text-danger";
  if (tone === "accent") return "text-accent";
  return "text-muted";
}
</script>

<template>
  <main class="max-w-120 md:max-w-150 mx-auto p-4" role="main">
    <BaseHeader
      :back-fallback="{ name: 'settings' }"
      :title="t('permissions.title')"
      spacing="sm"
    />

    <p class="intro">{{ t("permissions.intro") }}</p>

    <!-- Adjustable permissions (Android only — these are the surfaces the user
         can actually flip, and the ones Android may have suppressed after two
         denials). Hidden on desktop where there's nothing to adjust. -->
    <template v-if="secureAvailable">
      <p class="group-label">{{ t("permissions.groups.adjustable") }}</p>

      <div class="cards">
        <BaseCard as="section">
          <div
            class="perm-row"
            :class="{ 'perm-tappable': notifTap }"
            :role="notifTap ? 'button' : undefined"
            :tabindex="notifTap ? 0 : undefined"
            :aria-label="notifAria"
            @click="notifTap && openNotifications()"
            @keydown.enter="notifTap && openNotifications()"
            @keydown.space.prevent="notifTap && openNotifications()"
          >
            <BaseIcon :icon="Bell" :size="20" class="text-muted" />
            <div class="perm-text">
              <h2 class="perm-title">
                {{ t("permissions.notifications.title") }}
              </h2>
              <p class="perm-body">{{ t("permissions.notifications.body") }}</p>
            </div>
            <div class="perm-trailing">
              <BaseSpinner v-if="!notifStatus" :size="14" />
              <span
                v-else
                class="perm-status"
                :class="toneClass(notifStatus.tone)"
                >{{ notifStatus.text }}</span
              >
              <BaseIcon
                v-if="notifTap"
                :icon="ChevronRight"
                :size="18"
                class="text-muted"
              />
            </div>
          </div>
        </BaseCard>

        <BaseCard as="section">
          <div
            class="perm-row"
            :class="{ 'perm-tappable': bioTap }"
            :role="bioTap ? 'button' : undefined"
            :tabindex="bioTap ? 0 : undefined"
            :aria-label="bioAria"
            @click="bioTap && openBiometric()"
            @keydown.enter="bioTap && openBiometric()"
            @keydown.space.prevent="bioTap && openBiometric()"
          >
            <BaseIcon :icon="Fingerprint" :size="20" class="text-muted" />
            <div class="perm-text">
              <h2 class="perm-title">{{ t("permissions.biometric.title") }}</h2>
              <p class="perm-body">{{ t("permissions.biometric.body") }}</p>
            </div>
            <div class="perm-trailing">
              <BaseSpinner v-if="!bioStatus" :size="14" />
              <span
                v-else
                class="perm-status"
                :class="toneClass(bioStatus.tone)"
                >{{ bioStatus.text }}</span
              >
              <BaseIcon
                v-if="bioTap"
                :icon="ChevronRight"
                :size="18"
                class="text-muted"
              />
            </div>
          </div>
        </BaseCard>
      </div>
    </template>

    <!-- Informational data-access notes (both platforms). No trailing affordance
         — these surfaces have no permission toggle the user can flip. -->
    <p class="group-label">{{ t("permissions.groups.informational") }}</p>

    <div class="cards">
      <BaseCard as="section">
        <div class="perm-row">
          <BaseIcon :icon="Clipboard" :size="20" class="text-muted" />
          <div class="perm-text">
            <h2 class="perm-title">{{ t("permissions.clipboard.title") }}</h2>
            <p class="perm-body">{{ t("permissions.clipboard.body") }}</p>
          </div>
        </div>
      </BaseCard>

      <BaseCard as="section">
        <div class="perm-row">
          <BaseIcon :icon="Globe" :size="20" class="text-muted" />
          <div class="perm-text">
            <h2 class="perm-title">{{ t("permissions.network.title") }}</h2>
            <p class="perm-body">{{ t("permissions.network.body") }}</p>
          </div>
        </div>
      </BaseCard>

      <BaseCard as="section">
        <div class="perm-row">
          <BaseIcon :icon="FileText" :size="20" class="text-muted" />
          <div class="perm-text">
            <h2 class="perm-title">{{ t("permissions.files.title") }}</h2>
            <p class="perm-body">{{ t("permissions.files.body") }}</p>
          </div>
        </div>
      </BaseCard>
    </div>
  </main>
</template>

<style scoped>
.intro {
  font-size: var(--text-sm);
  color: var(--color-muted);
  margin-bottom: 1rem;
}
.cards {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}
.group-label {
  font-size: 0.7rem;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--color-muted);
  margin: 1rem 0.5rem 0.4rem;
}
.perm-row {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  min-height: 3rem; /* 48px touch target, matching the Settings hub row */
  border-radius: var(--radius-sm);
}
.perm-tappable {
  cursor: pointer;
  transition: background-color 0.15s;
}
.perm-tappable:focus-visible {
  background: var(--color-hover, var(--color-edge));
  outline: none;
}
@media (hover: hover) {
  .perm-tappable:hover {
    background: var(--color-hover, var(--color-edge));
  }
}
.perm-text {
  flex: 1;
  min-width: 0;
}
.perm-title {
  font-size: 0.95rem;
  font-weight: 500;
}
.perm-body {
  font-size: var(--text-sm);
  color: var(--color-muted);
  margin-top: 0.15rem;
}
.perm-trailing {
  margin-left: auto;
  display: flex;
  align-items: center;
  gap: 0.25rem;
  flex-shrink: 0;
}
.perm-status {
  font-size: var(--text-sm);
  text-align: right;
}
</style>
