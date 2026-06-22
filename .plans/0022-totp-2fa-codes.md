# TOTP / 2FA codes

**Priority:** P1
**Status:** Draft
**Phase:** Next

## What

gpm can store and reveal a secret's password and notes but cannot surface a
rotating two-factor code. Add the ability to read a TOTP seed stored in a
secret's body and produce the current one-time code, surfaced and copied
without revealing the rest of the secret.

## Why

A password manager that holds the login but not the rotating 2FA code forces a
second app for every account that has 2FA — the single most common reason a
credential lookup is not self-contained. gopass has `gopass otp`; gpm has no
equivalent.

There is also a security angle. Today, revealing a secret sends its entire body
to the WebView, so a TOTP seed embedded in the notes already crosses into the
WebView whenever the user shows the secret. A dedicated TOTP path that computes
the code backend-side and returns only the short digit string is strictly safer
than the status quo, because the seed never needs to leave the trusted layer.

## Context

gpm's secret body is freeform text after the password line, and a TOTP seed is
conventionally stored as an `otpauth://totp/...` URI somewhere in that body —
the format gopass, Bitwarden, and Authenticator apps exchange. Generating a
TOTP code is a deterministic HMAC computation over the current time: no
randomness, no network.

Because generating a code requires decryption, the affordance naturally lives
where the store already decrypts — the detail view — not on the entry list,
which deliberately avoids decrypting every entry. The seed is a secret in its
own right, so it stays in the trusted backend: only the resulting one-time code
reaches the WebView, and it rides the same short-lived reveal / auto-clear
contract the password already uses.

The minimal, most conservative shape is a copy action that mirrors the existing
copy-password operation — secret material stays out of the WebView entirely.
Optionally, a displayed code with a countdown is a follow-on; it forces either
periodic re-computation or accepting a code that goes stale within the existing
reveal window, so it is not the first cut.

This composes cleanly with the existing biometric / auto-lock and
clipboard-auto-clear machinery, and is independent of the write-path work.

## Alternatives considered

- **Compute the code in the WebView from the revealed body.** Rejected: the body
  already reaches the WebView on reveal, but a TOTP path should not depend on a
  full reveal — it should let the user grab a 2FA code without exposing the
  password and notes, and it should keep the seed backend-side. Computing in the
  trusted layer and returning only the digit string is strictly safer.
- **Detect TOTP on the entry list (a badge per entry).** Rejected for now:
  detection requires decryption, which the list path deliberately avoids. A
  list-level badge would need a sidecar metadata convention; defer. The detail
  view — where decryption already happens — is the natural first home.
- **Live-displayed code with a countdown, from the start.** Rejected as the
  first cut: it forces either periodic re-invocation or accepting a code that
  goes stale inside the existing reveal window. Start with the copy action; the
  countdown view is a follow-on.
- **Pull in a heavyweight TOTP library.** Rejected: HOTP/TOTP is a short,
  well-understood HMAC truncation; a tiny dependency footprint (HMAC, one SHA
  variant, base32 decoding) is sufficient.

## Effort

~0.5 day (human) / ~15 min (CC). Small, deterministic crypto over an
already-decrypted body, reusing the existing copy / reveal / clipboard-clear
path.

## Depends on / Supersedes

None. Composes with the existing reveal / auto-clear and clipboard machinery,
and is independent of the write-path RFCs. Sits naturally alongside
`0011-password-generator.md` as part of completing the create/use loop, but has
no hard dependency on it.
