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
import { beforeAll, describe, expect, it } from "vitest";

/**
 * The native-prompt-text builders are the single point that maps each native
 * surface to its `native.json` keys, so a key rename or a missing bundle
 * surfaces here rather than as an untranslated prompt on-device.
 *
 * This vitest config enables `@intlify/unplugin-vue-i18n` precompilation (as
 * the production `vite build` does), so the assertions exercise the real
 * compiled-message shapes. That matters for the clipboard body: a naive `tm()`
 * returns the compiled AST object, not the raw string — passing that object
 * through IPC makes the backend's `body_template: Option<String>` reject it
 * (`invalid type: map, expected a string`). The builder must therefore resolve
 * to a STRING with the `{secs}` hole intact for Rust to substitute at post time.
 */
describe("native prompt-text builders", () => {
  beforeAll(async () => {
    i18n.global.locale.value = "en";
    await loadBundle("en", "native");
  });

  it("identity biometric builders read the en bundle", () => {
    expect(identityEnrollPrompt()).toEqual({
      title: "Enable biometric unlock",
      subtitle: "Authenticate to access gpm",
      negative: "Use passphrase",
    });
    expect(identityUnlockPrompt().title).toBe("Unlock gpm");
  });

  it("app-lock biometric builders read the en bundle", () => {
    expect(appLockEnrollPrompt().title).toBe("Enable app lock");
    expect(appLockUnlockPrompt().negative).toBe("Cancel");
  });

  it("clipboard builder returns STRING fields with the {secs} hole intact", () => {
    const t = clipboardNotifyText();
    // Every field must serialize as a string (or undefined) — a compiled-message
    // object here would cross IPC as a map and fail the backend's Option<String>.
    // This is the regression guard for the precompilation `tm`-vs-`t` bug.
    expect(JSON.parse(JSON.stringify(t))).toEqual(t);
    for (const v of Object.values(t)) {
      expect(v === undefined || typeof v === "string").toBe(true);
    }
    expect(t.title).toBe("gpm");
    expect(t.bodyTemplate).toBe("Tap to clear · auto-clears in {secs}s");
    expect(t.channelName).toBe("Clipboard");
    expect(t.channelDescription).toBe(
      "Notifies when a secret is on the clipboard so you can clear it",
    );
  });

  it("every locale's clipboard body template carries the {secs} hole", async () => {
    // A translator writing `{sec}`/`{SECS}` or dropping the token would otherwise
    // ship undetected — the body would render the literal token or omit the
    // number. Rust substitutes `{secs}` at post time, so the token is the contract.
    for (const locale of SUPPORTED_LOCALES) {
      i18n.global.locale.value = locale;
      await loadBundle(locale, "native");
      // Resolve via `t` with the sentinel (mirrors `clipboardNotifyText`); `tm`
      // returns the compiled AST object under precompilation, not a string.
      const body = i18n.global.t("native.clipboard.autoClearBody", {
        secs: "{secs}",
      });
      expect(body, `${locale} autoClearBody`).toContain("{secs}");
    }
  });
});
