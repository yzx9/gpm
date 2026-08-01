<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

<script setup lang="ts">
// Unified page header. One rule across the app: the LEFT side is up/back
// navigation (the root shows its logo via the `#nav` slot; every sub-page shows
// a back button), the RIGHT side is contextual forward actions (`#actions`).
//
// The back button is icon-only (ArrowLeft, no "Back" text) with aria-label, so
// it stays accessible without competing with page actions. It pops the nav
// stack via `navBack` — the same logic every page used to hand-roll — so back
// *behavior* is unified alongside back *placement*.
//
// Title is intentionally NOT forced into one style: the common "icon + label"
// title renders from `title`/`titleIcon` props, while pages with custom title
// chrome (text-lg, truncating entry names) pass their own <h1> via `#title`,
// which overrides the prop.
import { navBack } from "@/utils/nav";
import type { LucideIcon } from "@lucide/vue";
import { ArrowLeft } from "@lucide/vue";
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import type { RouteLocationRaw } from "vue-router";
import { useRouter } from "vue-router";
import BaseIcon from "./BaseIcon.vue";

const props = withDefaults(
  defineProps<{
    /** When set, renders an icon-only Back button on the LEFT that calls
     *  navBack(router, backFallback). Mutually exclusive with the `#nav` slot:
     *  supplying `#nav` replaces the whole left cluster, so the back button
     *  won't render even when this is set. Omit on the root (logo + badge). */
    backFallback?: RouteLocationRaw;
    /** Bottom margin: "sm" (mb-4, root + history) or "md" (mb-6, default). */
    spacing?: "sm" | "md";
    /** Canonical "icon + label" title. Rendered as <h1>; overridden by #title. */
    title?: string;
    /** Leading icon for the `title` prop. */
    titleIcon?: LucideIcon;
  }>(),
  { spacing: "md" },
);

const emit = defineEmits<{
  /** Fired on back-button click BEFORE navBack runs. Side-effects only (e.g. a
   *  page wipe). The handler MUST NOT navigate — BaseHeader owns the nav, and a
   *  second nav here would race it. */
  (e: "back"): void;
}>();

const router = useRouter();
const { t } = useI18n();

const spacingClass = computed(() => (props.spacing === "sm" ? "mb-4" : "mb-6"));

function onBack() {
  emit("back");
  // backFallback is guaranteed set when the button is rendered (v-if), but read
  // it through a guard rather than a non-null assertion.
  if (props.backFallback) navBack(router, props.backFallback);
}
</script>

<template>
  <header
    class="flex justify-between items-center gap-3"
    :class="spacingClass"
    role="banner"
  >
    <!-- Left cluster: #nav overrides the whole default (root logo + badge);
         otherwise it's the back button (if any) + the title. -->
    <slot name="nav">
      <div class="flex items-center gap-3 min-w-0 flex-1">
        <button
          v-if="backFallback"
          type="button"
          class="base-header__back"
          :aria-label="t('common.back')"
          @click="onBack"
        >
          <BaseIcon :icon="ArrowLeft" />
        </button>
        <slot name="title">
          <h1 v-if="title" class="text-xl flex items-center gap-1">
            <BaseIcon v-if="titleIcon" :icon="titleIcon" :size="24" />
            {{ title }}
          </h1>
        </slot>
      </div>
    </slot>

    <!-- Right cluster: contextual forward actions. -->
    <div v-if="$slots.actions" class="flex gap-2 items-center shrink-0">
      <slot name="actions" />
    </div>
  </header>
</template>

<style scoped>
/* Icon-only back affordance: transparent (reads as a link, not a chip),
   accent-colored, ≥48px touch target, press feedback to a deeper accent.
   aria-label supplies the accessible name; the icon self-marks aria-hidden. */
.base-header__back {
  background: transparent;
  border: none;
  cursor: pointer;
  color: var(--color-accent);
  min-width: 48px;
  min-height: 48px;
  padding: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--radius-sm);
  -webkit-tap-highlight-color: transparent;
}
.base-header__back:active {
  color: var(--color-accent-deep);
}
@media (hover: hover) {
  .base-header__back:hover {
    color: var(--color-accent-deep);
  }
}
.base-header__back:focus-visible {
  outline: 2px solid var(--color-accent);
  outline-offset: 2px;
}
</style>
