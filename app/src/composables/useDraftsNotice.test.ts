// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { describe, expect, it } from "vitest";
import { createDraftsNotice } from "./useDraftsNotice";

describe("createDraftsNotice", () => {
  it("consume() is false until mark()", () => {
    const n = createDraftsNotice();
    expect(n.consume()).toBe(false);
  });

  it("mark() then consume() is true — read-and-reset", () => {
    const n = createDraftsNotice();
    n.mark();
    expect(n.consume()).toBe(true);
    // Reset by the consume itself: a second consume (the other lock's unlock
    // edge in the same cycle) must not fire a second toast.
    expect(n.consume()).toBe(false);
  });

  it("mark() is idempotent within a cycle", () => {
    const n = createDraftsNotice();
    n.mark();
    n.mark();
    expect(n.consume()).toBe(true);
    expect(n.consume()).toBe(false);
  });
});
