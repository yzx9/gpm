// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import type { CommitSigStatus } from "@/api";
import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import CommitSigIndicator from "./CommitSigIndicator.vue";

const VERIFIED: CommitSigStatus = { kind: "verified", signer_fp: "AB:CD" };
const UNTRUSTED: CommitSigStatus = {
  kind: "untrusted_key",
  signer_fp: "EF:12",
};
const UNSIGNED: CommitSigStatus = { kind: "unsigned" };
const BAD: CommitSigStatus = { kind: "bad_signature" };
const UNSUPPORTED: CommitSigStatus = {
  kind: "unsupported_format",
  format: "x509",
};
const UNKNOWN: CommitSigStatus = { kind: "unknown" };

describe("CommitSigIndicator — glyph variant (default)", () => {
  it("renders ✓ with the success tone for verified", () => {
    const w = mount(CommitSigIndicator, { props: { status: VERIFIED } });
    expect(w.text()).toBe("✓");
    expect(w.classes()).toContain("text-success");
  });

  it("renders ⛔ with the danger tone for bad_signature", () => {
    const w = mount(CommitSigIndicator, { props: { status: BAD } });
    expect(w.text()).toBe("⛔");
    expect(w.classes()).toContain("text-danger");
  });

  it("uses the warning tone for unsigned / untrusted / unsupported / unknown", () => {
    for (const status of [UNSIGNED, UNTRUSTED, UNSUPPORTED, UNKNOWN]) {
      const w = mount(CommitSigIndicator, { props: { status } });
      expect(w.classes()).toContain("text-warning");
      expect(w.classes()).not.toContain("text-success");
      expect(w.classes()).not.toContain("text-danger");
    }
  });

  it("renders ? for unsupported_format and unknown", () => {
    expect(
      mount(CommitSigIndicator, { props: { status: UNSUPPORTED } }).text(),
    ).toBe("?");
    expect(
      mount(CommitSigIndicator, { props: { status: UNKNOWN } }).text(),
    ).toBe("?");
  });

  it("falls caller-supplied classes onto the glyph root (attribute fallthrough)", () => {
    // HistoryPage's list row depends on these landing on the rendered span.
    const w = mount(CommitSigIndicator, {
      props: { status: VERIFIED },
      attrs: { class: "w-6 text-center shrink-0" },
    });
    expect(w.classes()).toContain("w-6");
    expect(w.classes()).toContain("text-center");
    expect(w.classes()).toContain("shrink-0");
  });

  it("hides the glyph from screen readers (aria-hidden)", () => {
    const w = mount(CommitSigIndicator, { props: { status: VERIFIED } });
    expect(w.attributes("aria-hidden")).toBe("true");
  });
});

describe("CommitSigIndicator — banner variant", () => {
  it("renders label + fingerprint on the success background for verified", () => {
    const w = mount(CommitSigIndicator, {
      props: { status: VERIFIED, variant: "banner" },
    });
    expect(w.classes()).toContain("bg-success-soft");
    expect(w.classes()).toContain("text-success");
    expect(w.text()).toContain("Verified");
    expect(w.text()).toContain("AB:CD");
  });

  it("renders the fingerprint for untrusted_key", () => {
    const w = mount(CommitSigIndicator, {
      props: { status: UNTRUSTED, variant: "banner" },
    });
    expect(w.text()).toContain("Untrusted signer");
    expect(w.text()).toContain("EF:12");
  });

  it("omits the fingerprint row when the status carries none", () => {
    const w = mount(CommitSigIndicator, {
      props: { status: UNSIGNED, variant: "banner" },
    });
    expect(w.text()).toContain("Unsigned");
    expect(w.text()).not.toContain("AB:CD");
    expect(w.text()).not.toContain("EF:12");
  });

  it("uses the danger background for bad_signature", () => {
    const w = mount(CommitSigIndicator, {
      props: { status: BAD, variant: "banner" },
    });
    expect(w.classes()).toContain("bg-danger-soft");
    expect(w.classes()).toContain("text-danger");
    expect(w.text()).toContain("Bad signature");
  });

  it("interpolates the format into the label for unsupported_format", () => {
    const w = mount(CommitSigIndicator, {
      props: { status: UNSUPPORTED, variant: "banner" },
    });
    expect(w.text()).toContain("Unsupported (x509)");
  });

  it("shows the ignored chip only when ignored=true", () => {
    const hidden = mount(CommitSigIndicator, {
      props: { status: UNSIGNED, variant: "banner", ignored: false },
    });
    expect(hidden.text()).not.toContain("ignored");

    const shown = mount(CommitSigIndicator, {
      props: { status: UNSIGNED, variant: "banner", ignored: true },
    });
    expect(shown.text()).toContain("ignored");
  });

  it("renders the fingerprint with an overflow guard for long fingerprints", () => {
    const w = mount(CommitSigIndicator, {
      props: { status: VERIFIED, variant: "banner" },
    });
    const fp = w.find(".break-all");
    expect(fp.exists()).toBe(true);
    expect(fp.classes()).toContain("text-muted");
  });

  it("falls caller-supplied classes onto the banner root (attribute fallthrough)", () => {
    const w = mount(CommitSigIndicator, {
      props: { status: UNSIGNED, variant: "banner" },
      attrs: { class: "mt-3" },
    });
    expect(w.classes()).toContain("mt-3");
  });
});
