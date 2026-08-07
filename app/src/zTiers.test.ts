// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { describe, expect, it } from "vitest";
import { Z } from "./zTiers";

describe("Z tiers", () => {
  // The whole overlay-stacking design rests on this ordering: a toast fired
  // from behind an opaque gate (Z.gate) must stay visible, and every gate must
  // sit above ordinary overlays. Inverting any value silently breaks the
  // user-visible guarantee while every other test (which references tiers by
  // name) stays green — so pin the invariant by value here.
  it("keeps transient feedback above every opaque overlay (toast > gate > overlay > chrome)", () => {
    expect(Z.toast).toBeGreaterThan(Z.gate);
    expect(Z.gate).toBeGreaterThan(Z.overlay);
    expect(Z.overlay).toBeGreaterThan(Z.chrome);
  });
});
