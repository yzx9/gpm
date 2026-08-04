// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

/**
 * Overlay z-index tiers — the single source of truth for BOTH visual stacking
 * and Android back-handler routing (R062). Wide numeric gaps prevent collision.
 *
 * Overlays adopt these via `BaseModalShell`'s `z` prop (applied as an inline
 * `z-index`) and pass the same value into the back-handler registry, so what
 * paints on top and what a back press dismisses are decided by one number.
 *
 * - `overlay` — every modal/sheet/dialog shell (BaseModalShell default).
 * - `gate`    — the app-launch lock; must sit above every overlay regardless of
 *   where it lives in the component tree. This is the only cross-tier guarantee
 *   the back key needs — within `overlay`, stacking is by mount/DOM order and
 *   the registry's LIFO tie-break.
 * - `toast`   — transient toast feedback; sits above every overlay including
 *   `gate`, so a toast fired from behind a fullscreen gate (e.g. the lock
 *   screen's diagnostics export) stays visible. NOT a back-consumer.
 * - `chrome`  — sticky in-page chrome (tab bars, transient badges). NOT a
 *   back-consumer; included for completeness. Its CSS consumers (the sync
 *   badge, the About tab bar) still use raw `z-index` today — migrating them to
 *   `Z.chrome` is a follow-up (a behavior change, deferred from R062).
 */
export const Z = {
  chrome: 100,
  overlay: 1000,
  gate: 2000,
  /** Above `gate` so transient feedback is never hidden by an opaque overlay. */
  toast: 3000,
} as const;

export type ZTier = (typeof Z)[keyof typeof Z];
