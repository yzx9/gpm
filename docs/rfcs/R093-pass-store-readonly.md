# Read standard `pass` (passwordstore.org) stores

**Priority:** P2
**Status:** Draft
**Phase:** Next

## What

gpm opens gopass-GPG stores but rejects standard `pass` (passwordstore.org)
stores — a store that carries a `.gpg-id` recipient list but no gopass
`.public-keys/` directory. This RFC adds **read-only** support for standard
pass stores: relax the GPG backend's membership gate so that, when no
`.public-keys/` pool exists, membership is confirmed by matching the imported
OpenPGP secret key's fingerprint / key ID against the `.gpg-id` tokens directly.
Read / list / search / copy then work over the existing decrypt path, which is
already independent of `.public-keys/`. A pass store is never written in place.
Serves `docs/specs/004-encryption-gpg`.

## Why

The README claims a password-store repository "just works"; today that is false
for vanilla `pass` stores, because the membership gate assumes gopass's
`.public-keys/` invariant (every `.gpg-id` token has a matching armored public
key committed in the repo). `pass` is the canonical standard that gopass itself
reads, and being unable to open one is a real ecosystem-compat gap. Read-only
support lets a `pass` user reach their existing store from gpm — notably on
Android — with zero new trust surface (no keyserver, no system keyring), and
makes the README honest. It is also the prerequisite for any future migration
path: you must be able to read a store before you can re-encrypt it (R095).

## Context

**Where it breaks today.** Opening a GPG store runs a membership gate that
confirms the imported identity is one of the store's recipients. For gopass-GPG
stores the gate resolves each `.gpg-id` token to an armored public key in a
`.public-keys/` directory and compares primary fingerprints. Standard `pass`
stores carry only `.gpg-id` — identifiers (key ID, fingerprint, or rarely an
email) — and no `.public-keys/`, so the gate hard-errors before the user ever
reaches decryption.

**Reads do not need public keys.** OpenPGP decryption needs only the
recipient's secret key; `.gpg-id` and `.public-keys/` are about *who to encrypt
to*, not how to decrypt. The decrypt path is already independent of
`.public-keys/`, so the sole blocker is the membership gate itself.

**The relaxation.** When no `.public-keys/` pool exists, confirm membership by
deriving the imported secret key's primary fingerprint and key IDs (available
from public-packet data alone, no passphrase) and matching them against the
`.gpg-id` tokens. A key absent from `.gpg-id` is definitively rejected — it
cannot decrypt anything in the store. When a `.public-keys/` pool *does* exist,
today's gopass path is unchanged, so gopass-GPG stores are unaffected.

**gopass alignment.** gopass opens `pass` stores natively, but resolves
`.gpg-id` tokens through the system `gpg` binary and the `~/.gnupg` keyring —
exactly what gpm forbids (A006). gpm therefore reproduces gopass's *on-disk
format and recipient semantics* while substituting its bundled OpenPGP library
and the imported identity everywhere gopass shells out. The on-disk store is
left untouched (gpm adds nothing, writes nothing), so `pass` and gpm can
coexist on the same repo.

**No write.** A `pass` store stays strictly read-only in gpm. Encrypting a new
secret would require resolving *all* `.gpg-id` recipients' public keys, which
gpm cannot do without a keyring or keyserver. Write-to-a-`pass`-store is
deliberately out of scope; the route to writability is conversion (R095), which
re-encrypts into a gpm-native store.

**Known gap.** `.gpg-id` tokens expressed as an email or user ID (rather than a
key ID or fingerprint) are not matched by fingerprint / key-ID comparison; a
follow-up can add a user-ID match pass. The common `pass init <fingerprint>` and
`pass init <keyid>` forms all work.

## Alternatives considered

- **Add a system-keyring fallback.** Rejected: A006 forbids reading `~/.gnupg`
  or invoking system `gpg`, which is the whole reason gpm cannot simply copy
  gopass.
- **Keyserver lookup to materialize `.public-keys/`.** Rejected for reads:
  decryption needs no public keys, so a keyserver would add a network trust
  surface for no read-path benefit. (A keyserver may matter for a future
  *write* path, not this RFC.)
- **Read-write in place (encrypt to self when the store is single-recipient).**
  Rejected as the v1 shape: it adds a conditional GPG write path with
  recipient-shrinkage hazards and competes with the cleaner "convert, then
  write" model (R095). Keep `pass` stores read-only; route writes through
  convert.
- **Silently treat a missing `.public-keys/` as "undetermined" and admit any
  key.** Rejected: the gate must still confirm the imported key is actually a
  recipient, else a wrong key is accepted for a store it cannot decrypt.

## Effort

Small (human ~1 day / CC ~15 min): the membership check gains a
no-`.public-keys/` fallback that matches the imported key's fingerprint / key-ID
against `.gpg-id` tokens, reusing existing fingerprint derivation; plus
gopass/`pass` interop tests against a real `pass init` store. No decrypt-path,
config, or caller changes.

## Depends on / Supersedes

Serves `docs/specs/004-encryption-gpg`. **Independent — no prerequisite.**
Multi-key read (stores whose sub-directories are encrypted to different keys you
hold several of) arrives for free once R094 (GPG identity management) lands;
until then single-identity read covers the common case.
