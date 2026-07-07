// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import type { CommitSigInfo } from "@/api";
import { mountWithApp } from "@/test/appTestUtils";
import { flushPromises } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import AuthenticityBlockModal from "./AuthenticityBlockModal.vue";

const unverifiedIssue: CommitSigInfo = {
  hash: "deadbeefdeadbeef",
  short_hash: "deadbee",
  author: "alice@example.com",
  date: "2026-01-01T00:00:00Z",
  subject: "Add something",
  status: { kind: "unverified_signature", signer_fp: "AB:CD:EF" },
  ignored: false,
};

describe("AuthenticityBlockModal", () => {
  it("renders the GPG notice with the offending commit hash and the settings link", async () => {
    const { wrapper } = mountWithApp(AuthenticityBlockModal, {
      mountOpts: { props: { issues: [unverifiedIssue] } },
    });
    await flushPromises();

    // The {hash} placeholder must render the commit's short hash. Regression
    // guard: an earlier <i18n-t> wiring left {hash} as an empty comment node
    // (the bare <code> was the default slot, not the #hash slot), dropping the
    // hash from the Enforce-mode "sync blocked" notice.
    expect(wrapper.text()).toContain("deadbee");
    expect(wrapper.text()).toContain("Settings → Trusted signing keys");
  });
});
