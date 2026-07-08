// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { i18n, loadBundle, SUPPORTED_LOCALES } from "@/i18n";
import {
  appLockEnrollPrompt,
  appLockUnlockPrompt,
  clipboardNotifyText,
  identityEnrollPrompt,
  identityUnlockPrompt,
} from "@/i18n/native";
import enNative from "@/locales/en/native.json";
import { beforeAll, describe, expect, it } from "vitest";

/**
 * The native-prompt-text builders are the single point that maps each
 * native surface to its `native.json` keys, so a key rename or a missing bundle
 * surfaces here rather than as an untranslated prompt on-device. Also pins the
 * clipboard `{secs}` contract: the body is read RAW (`tm`, not `t`) so the hole
 * survives for Rust to substitute at post time.
 */
describe("native prompt-text builders", () => {
  beforeAll(async () => {
    i18n.global.locale.value = "en";
    await loadBundle("en", "native");
  });

  it("identity biometric builders read the en bundle", () => {
    expect(identityEnrollPrompt()).toEqual({
      title: enNative.biometric.identity.enrollTitle,
      subtitle: enNative.biometric.subtitle,
      negative: enNative.biometric.identity.negative,
    });
    expect(identityUnlockPrompt().title).toBe(
      enNative.biometric.identity.unlockTitle,
    );
  });

  it("app-lock biometric builders read the en bundle", () => {
    expect(appLockEnrollPrompt().title).toBe(
      enNative.biometric.appLock.enrollTitle,
    );
    expect(appLockUnlockPrompt().negative).toBe(
      enNative.biometric.appLock.negative,
    );
  });

  it("clipboard builder returns the RAW body template with the {secs} hole intact", () => {
    const t = clipboardNotifyText();
    expect(t.title).toBe(enNative.clipboard.title);
    expect(t.bodyTemplate).toBe(enNative.clipboard.autoClearBody);
    // The hole must survive so Rust can substitute secs at post time.
    expect(t.bodyTemplate).toContain("{secs}");
    expect(t.channelName).toBe(enNative.clipboard.channelName);
    expect(t.channelDescription).toBe(enNative.clipboard.channelDescription);
  });

  it("every locale's clipboard body template carries the {secs} hole", async () => {
    // A translator writing `{sec}`/`{SECS}` or dropping the token would otherwise
    // ship undetected — the body would render the literal token or omit the
    // number. Rust substitutes `{secs}` at post time, so the token is the contract.
    for (const locale of SUPPORTED_LOCALES) {
      i18n.global.locale.value = locale;
      await loadBundle(locale, "native");
      const body = i18n.global.tm("native.clipboard.autoClearBody") as string;
      expect(body, `${locale} autoClearBody`).toContain("{secs}");
    }
  });
});
