# Reject plaintext identities when enabling biometric unlock

**Priority:** P3 (defense-in-depth; no live bug — the UI gate already prevents it today)
**Status:** Draft
**Phase:** Next

## What

Enabling biometric unlock should refuse a plaintext (passphrase-less) identity
in the **backend**, not rely on the frontend to hide the option. Add a guard at
the biometric-enable command so it rejects a non-encrypted identity before
anything is sealed into the Keystore.

## Why

Biometric unlock exists to gate a real passphrase — it seals the identity
passphrase in the Android Keystore and retrieves it through a biometric prompt.
That is meaningless for a plaintext identity, which has no passphrase. Today the
only thing stopping a user from "enabling biometric" on a plaintext identity is
that the Settings screen hides the control unless the identity is encrypted — a
presentation guard with no backend backstop. If that UI gate ever regresses, the
backend would happily seal a nothing-useful passphrase into the Keystore and the
lock / biometric-retrieve flow would behave oddly. The backend owns every other
security invariant (the passphrase never reaches the WebView, etc.), so this
guard belongs there too.

## Context

The enable flow validates the passphrase before sealing it — but that validation
is a no-op for plaintext identities (there is no passphrase to check). So the
backend never distinguishes "enable biometric on an encrypted identity" from
"enable biometric on a plaintext identity"; only the UI does.

The store already exposes whether the identity is passphrase-encrypted, so the
fix is a backend check at the top of the enable command: refuse with a clear
error when the identity is not encrypted, before touching the Keystore. No
threat-model change and no capability change — biometric was never useful for
plaintext identities. This closes a latent inconsistency surfaced during the
0013 / 0014 eng review.

## Alternatives considered

- **Leave it — the UI gate works.** Rejected: single-point presentation guard,
  no backend backstop, inconsistent with where every other invariant lives.
- **Frontend-only check.** Rejected for the same reason; the backend is the
  source of truth for security here.
- **Make the validation step itself reject plaintext.** Rejected — that step is
  intentionally a no-op success for plaintext (it is reused by flows that
  legitimately accept plaintext). The refusal belongs at the biometric-enable
  boundary, not inside the shared validator.

## Effort

~S (human: ~30 min / CC: ~5 min) — one backend guard plus a test that enabling
biometric on a plaintext identity is refused.

## Depends on / Supersedes

Surfaced during the 0013 / 0014 eng review (outside-voice call-site check).
Independent of 0013 / 0014 — can land on its own.
