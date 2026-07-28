# Recipients pinning + acknowledge

**Priority:** P2
**Status:** Draft
**Phase:** Next

## What

Locally pin a hash of the store's recipients file and refuse to encrypt new secrets to a changed list until the user explicitly acknowledges it — mirroring gopass's `recipients ack` / `recipients.hash`. On sync, drift between the pinned hash and the recipients file is surfaced (non-blocking); on the secret-create path, a mismatch blocks the write and routes the user to review + acknowledge.

## Why

The recipients file is the one artifact a remote adversary can tamper in the shared git store to get themselves added to future-encrypted secrets: if their key lands in the list, every secret created afterward gets encrypted to them too. gpm's commit-signature verification covers this only in Audit/Enforce mode and only at commit granularity; a file-level pin is independent of signing mode, works when verification is Off, and is the faithful match for gopass's model. It is defense in depth that composes with (not duplicates) the existing authenticity feature.

## Context

gpm now encrypts (in-app secret creation), so the encrypt-to-attacker surface is real. The pin lives in local-only `repo.json`, next to the already-accepted `authenticity` trust set — consistent with the SECURITY.md assumption that no local attacker has write access to private storage: a _remote_ attacker can change the synced recipients file but cannot rewrite local `repo.json`, so drift is detectable. First use auto-pins the current hash (TOFU, like trusting HEAD's signer); an explicit acknowledge re-pins after a legitimate change. Sync surfaces drift without blocking (Audit philosophy); the write path blocks on mismatch until acknowledged. The hash is over the resolved recipients file exactly as the recipient parser reads it. Prior art: gopass `recipients.hash` / `recipients ack`.

## Alternatives considered

- **Rely solely on commit-signature verification.** Rejected: no coverage in `Off` mode (the default), and it gives no file-level signal independent of how the file changed.
- **Make acknowledge a blocking gate on sync.** Rejected: too disruptive for legitimate teammate changes; mirrors Audit's surface-don't-block philosophy, with the hard gate reserved for the write path where it is security-critical.
- **Do nothing.** Rejected: the write path can encrypt to an injected recipient, which is the exact attack gopass built this against.

## Effort

~1 day (human) / ~30 min (CC)

## Depends on / Supersedes

Composes with `0009-gpg-signature-verification.md` (independent file-level check on top of commit-level verification); does not duplicate it.
