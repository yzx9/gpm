<!-- SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz> -->
<!-- -->
<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->

<script setup lang="ts">
import { computed } from "vue";
import BaseSpinner from "./BaseSpinner.vue";

const props = withDefaults(
  defineProps<{
    /** Visual style. `ghost` is a quiet tinted action (no border, subordinate to
     * `primary`) for low-emphasis entries like a method-switch; `action` /
     * `action-danger` are full-width, left-aligned; `link` is a transparent,
     * borderless affordance whose color is set by `tone` (close-X, eye-toggle,
     * back arrow, text links). */
    variant?:
      | "primary"
      | "secondary"
      | "outline"
      | "ghost"
      | "danger"
      | "action"
      | "action-danger"
      | "link";
    /** `md` (default), `sm`, or `xs` (36px inline chips like copy buttons).
     * Ignored for `action` variants (fixed compact size). */
    size?: "md" | "sm" | "xs";
    /** Foreground color for the `link` variant only (other variants define their
     * own foreground). `default` inherits the surrounding text color. */
    tone?: "default" | "muted" | "accent" | "danger" | "success" | "warn";
    /** Shows a leading spinner (white on `primary`, dark elsewhere) and disables. */
    loading?: boolean;
    /** Stretch to 100% width. */
    block?: boolean;
    /** Native button type. */
    type?: "button" | "submit" | "reset";
    /** Forwarded to the native button; also forced on when `loading`. */
    disabled?: boolean;
  }>(),
  {
    variant: "secondary",
    size: "md",
    tone: "default",
    loading: false,
    block: false,
    type: "button",
    disabled: false,
  },
);

const sizeClass = computed(() =>
  props.variant === "action" || props.variant === "action-danger"
    ? null
    : `size-${props.size}`,
);

/** `tone` only colors the `link` variant — other variants own their foreground,
 * and a tone class would fight them at equal specificity. */
const toneClass = computed(() =>
  props.variant === "link" && props.tone !== "default"
    ? `tone-${props.tone}`
    : null,
);
</script>

<template>
  <button
    :type="type"
    class="btn"
    :class="[variant, sizeClass, toneClass, { block: block }]"
    :disabled="disabled || loading"
  >
    <BaseSpinner
      v-if="loading"
      :variant="
        variant === 'primary' || variant === 'danger' ? 'white' : 'dark'
      "
    />
    <slot />
  </button>
</template>

<style scoped>
/* Base layout. Original buttons were a mix of inline-block and flex; unifying on
   inline-flex + gap lets a leading spinner or icon align without per-variant
   tweaks. Visuals are preserved: auto-width buttons shrink to content, full-width
   variants set width:100% themselves. */
.btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 0.4rem;
  font-family: inherit;
  font-weight: 500;
  cursor: pointer;
  transition: background 0.2s;
  min-height: 48px;
}
.btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}
/* Keyboard focus comes from the global `button:focus-visible` ring in
   style.css, so BaseButton matches every other button in the app — no
   per-component outline (that would stack a second ring on the global one). */

.block {
  width: 100%;
}

/* Sizes — min-height stays on the base `.btn` (48px); only `xs` overrides it
   down to 36px. `action*` variants carry no size class, so they keep inheriting
   the base 48px (guarded by the action-min-height regression test). */
.size-md {
  padding: 0.75rem;
  font-size: var(--text-md);
  border-radius: var(--radius-md);
}
.size-sm {
  padding: 0.5rem 0.75rem;
  font-size: var(--text-sm);
  border-radius: var(--radius-sm);
}
.size-xs {
  padding: 0.3rem 0.6rem;
  font-size: var(--text-xs);
  border-radius: var(--radius-sm);
  min-height: 36px;
}

/* Variants */
.primary {
  background: var(--color-accent);
  color: var(--color-on-accent);
  border: none;
}
/* Pressed (touch + mouse). BaseButton renders a real <button>, so :active is
   reliable here — this variant block is the reference pattern. The native tap
   flash is dropped globally (src/style.css) so the themed :active reads clean. */
.primary:active:not(:disabled) {
  background: var(--color-accent-deep);
}

.secondary {
  background: var(--color-surface);
  color: inherit;
  border: 1px solid var(--color-edge);
}
.secondary:active:not(:disabled) {
  background: var(--color-hover);
}

/* Accent-outlined secondary (e.g. entry-detail actions). */
.outline {
  background: var(--color-surface);
  color: var(--color-accent);
  border: 1px solid var(--color-accent);
}
.outline:active:not(:disabled) {
  background: var(--color-hover);
}

/* Quiet tinted action: a faint surface tint (no border) so the row reads as
   tappable on a no-hover touch screen, while staying clearly subordinate to a
   filled `primary`. The label uses the inherited body color so it clears
   contrast against the tint; subordination comes from no-fill + no-border. */
.ghost {
  background: var(--color-hover);
  color: inherit;
  border: none;
}
.ghost:active:not(:disabled) {
  background: var(--color-edge);
}

/* Filled destructive primary (mirrors `.primary`): a solid red fill so the
   destructive action reads as the clear primary next to its outline/secondary
   cancel, instead of competing as a second outlined button. */
.danger {
  background: var(--color-danger);
  color: var(--color-on-accent);
  border: none;
}
.danger:active:not(:disabled) {
  background: var(--color-danger-deep);
}

/* Transparent, borderless affordance (close-X, eye-toggle, back arrow, text
   links). Color comes from `tone`; `default` inherits the surrounding text. */
.link {
  background: transparent;
  border: none;
}
.link:active:not(:disabled) {
  opacity: 0.7;
}

/* Action variants: full-width, left-aligned, compact (size prop ignored). */
.action,
.action-danger {
  width: 100%;
  justify-content: flex-start;
  text-align: left;
  padding: 0.5rem 0.75rem;
  font-size: var(--text-sm);
  border-radius: var(--radius-md);
  background: var(--color-surface);
  border: 1px solid var(--color-edge);
}
.action:active:not(:disabled),
.action-danger:active:not(:disabled) {
  background: var(--color-hover);
}
.action-danger {
  border-color: var(--color-danger-edge, var(--color-danger));
  color: var(--color-danger);
}

/* `tone` colors the `link` variant only (`default` emits no class and inherits). */
.tone-muted {
  color: var(--color-muted);
}
.tone-accent {
  color: var(--color-accent);
}
.tone-danger {
  color: var(--color-danger);
}
.tone-success {
  color: var(--color-success);
}
.tone-warn {
  color: var(--color-warning);
}

/* Hover hint — mouse/trackpad only. Gated by (hover: hover) so it never
   sticks on touch (the original sticky-hover bug). Same colors as :active. */
@media (hover: hover) {
  .primary:hover:not(:disabled) {
    background: var(--color-accent-deep);
  }
  .secondary:hover:not(:disabled),
  .outline:hover:not(:disabled),
  .action:hover:not(:disabled),
  .action-danger:hover:not(:disabled) {
    background: var(--color-hover);
  }
  .ghost:hover:not(:disabled) {
    color: var(--color-accent);
  }
  .danger:hover:not(:disabled) {
    background: var(--color-danger-deep);
  }
  .link:hover:not(:disabled) {
    opacity: 0.7;
  }
}
</style>
