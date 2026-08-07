<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<script setup lang="ts">
import {
  useOverlayBackHandler,
  useScrollLock,
  Z,
  type ZTier,
} from "@/composables";
import { computed, ref } from "vue";
import BaseCard from "./BaseCard.vue";

const props = withDefaults(
  defineProps<{
    /** `center` = always-centered gate (unlock/app-lock); `sheet` = bottom-sheet
     *  on mobile, centered ≥sm; `fullscreen` = an opaque edge-to-edge surface
     *  (the app-launch lock) that fully occludes underlying content — no card. */
    variant: "center" | "sheet" | "fullscreen";
    /** Stacking layer — a `Z` tier (`Z.overlay` default) or an explicit number.
     *  AppLockOverlay passes `Z.gate` to sit above every overlay. */
    z?: ZTier | number;
    /** Accessibility role; `dialog` (default) or `alertdialog`. */
    role?: string;
    /** Whether a backdrop tap emits `close` (default true). Set false to force a
     *  button-tap dismissal (e.g. a security-warning sheet that shouldn't be
     *  waved away by tapping outside). */
    dismissOnBackdrop?: boolean;
    /** Whether the Android back button emits `close` (default true). Set false
     *  to trap back — the listener still suppresses the default webview goBack —
     *  e.g. a stacked shell that shouldn't dismiss, or while an async resolve is
     *  in flight. */
    dismissOnBack?: boolean;
  }>(),
  { role: "dialog", dismissOnBackdrop: true, dismissOnBack: true },
);

const emit = defineEmits<{ (e: "close"): void }>();

const resolvedZ = computed(() => props.z ?? Z.overlay);

// Backdrop and back are decoupled: each emits `close` only when its own prop
// allows it. A caller without `@close` still gets no dismissal (emit is a
// no-op with no listener), and back is always registered so the default
// webview goBack is suppressed for the lifetime of any mounted shell.
function onBackdrop() {
  if (props.dismissOnBackdrop) emit("close");
}

// The shell is mounted only when shown (every caller uses v-if), so a constant
// `ref(true)` arms the handler for the component's lifetime and
// onBeforeUnmount pops it. `resolvedZ.value` is the back-routing tier (stable
// for the shell's life). See useBackHandlerRegistry for the dispatch rule and
// stale-registration guards.
useOverlayBackHandler(
  ref(true),
  () => {
    if (props.dismissOnBack) emit("close");
  },
  resolvedZ.value,
);

// Freeze the document scroller for the shell's lifetime. The backdrop covers
// the viewport visually, but a drag on a non-scrolling fixed layer still
// scrolls the document behind it on touch WebViews — this locks the list/etc.
// under every modal (unlock, app-lock, divergence, …) until the shell unmounts.
// See useScrollLock for why `overflow: hidden` on documentElement and not
// `touch-action: none` (which would also freeze inner modal scroll regions).
useScrollLock();
</script>

<template>
  <div
    class="overlay"
    :class="variant"
    :style="{ zIndex: resolvedZ }"
    :role="role"
    aria-modal="true"
    @click.self="onBackdrop"
  >
    <div class="wrap" :class="variant">
      <BaseCard
        v-if="variant !== 'fullscreen'"
        :variant="variant === 'center' ? 'raised' : 'flat'"
      >
        <slot />
      </BaseCard>
      <!-- fullscreen renders the slot directly on the opaque surface — a card
           frame would re-introduce the "floating dialog" look the gate replaces,
           and the slot owns its own center/footer flex layout. -->
      <slot v-else />
    </div>
  </div>
</template>

<style scoped>
.overlay {
  position: fixed;
  inset: 0;
  display: flex;
  justify-content: center;
  background: rgba(0, 0, 0, 0.4);
}

/* Centered gate: always centered, honors notch/gesture insets, traps scroll. */
.center {
  align-items: center;
  padding: 1rem;
  padding-top: calc(1rem + var(--safe-area-inset-top, 0px));
  padding-bottom: calc(1rem + var(--safe-area-inset-bottom, 0px));
  overscroll-behavior: contain;
}

/* Bottom sheet: docked to the bottom on mobile, centered on ≥640px. Like .center,
   it honors the bottom safe-area inset so the last row clears the gesture bar. */
.sheet {
  align-items: center;
  padding: 1rem;
  padding-bottom: calc(1rem + var(--safe-area-inset-bottom, 0px));
}
@media (max-width: 639px) {
  .sheet {
    align-items: flex-end;
  }
}

/* Fullscreen opaque gate: a solid page-bg surface that bleeds edge-to-edge and
   fully occludes underlying app content (the app-launch lock). `.overlay.fullscreen`
   wins over the base `.overlay`'s translucent backdrop on specificity. The
   surface covers the whole viewport (notch + gesture bar via safe-area padding);
   only the inner content column is width-constrained. */
.overlay.fullscreen {
  background: var(--color-page);
  /* stretch so .wrap fills the viewport height and the slot's footer can pin to
     the bottom; horizontal centering is inherited from the base `.overlay`. */
  align-items: stretch;
  padding: 1rem;
  padding-top: calc(1rem + var(--safe-area-inset-top, 0px));
  padding-bottom: calc(1rem + var(--safe-area-inset-bottom, 0px));
  overscroll-behavior: contain;
}

.wrap {
  width: 100%;
}
.wrap.center {
  max-width: 420px;
}
.wrap.sheet {
  max-width: 30rem; /* max-w-120 */
}
.wrap.fullscreen {
  /* a flex column so the slot's body can `flex: 1` (vertically center the logo
     / button) while a footer rides the bottom safe-area. */
  display: flex;
  flex-direction: column;
  width: 100%;
  max-width: 420px;
}
</style>
