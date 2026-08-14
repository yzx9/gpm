# Convert a standard `pass` store to a gpm-writable gopass-GPG store

**Priority:** P2
**Status:** Blocked
**Phase:** Future

## What

A one-shot migration that reads a standard `pass` store and re-encrypts every
secret into a gopass-GPG store gpm can both read and write — mirroring gopass's
`convert` and its `recipients add/remove` re-encryption. General (multi-key) by
design, not single-user-only: it decrypts each secret with the correct identity
per sub-directory and re-encrypts to the target recipient set. After conversion
the store is a normal gpm gopass-GPG store; the `pass` layout is left behind.
Serves `docs/specs/004-encryption-gpg`.

## Why

Read-only support (R093) lets a `pass` user _view_ their store in gpm; to
_create or edit_ secrets they must leave the `pass` format, because gpm cannot
encrypt to a `pass` store's recipient set without a system keyring (A006).
Conversion is that move. Doing it generally (multi-key) from the start avoids
shipping a single-recipient slice that would be reworked the moment a team or
rotated-key store appears — consistent with gpm's stance of building on proper
foundations rather than one-off slices.

## Context

**Two existing engines compose.** Decrypting every secret with the right
per-sub-directory identity is R094's targeted decryption; re-encrypting to a
recipient set and seeding `.public-keys/` is R083's recipient-management and
re-encryption machinery. Convert is a thin orchestration over both — it
introduces no new crypto of its own.

**Mirrors gopass.** gopass's `convert` decrypts with the old backend and
re-encrypts with the new; its `recipients add/remove` _eagerly_ re-encrypts the
whole store so a membership change takes effect immediately (lazy re-encryption
is rejected as silently inconsistent — a teammate discovers, at the worst time,
that they cannot read an entry). gpm matches the eager model for gopass
compatibility.

**Result is gopass-GPG, not `pass`.** The output carries `.public-keys/` and
gpm-managed recipients, so gpm can write it. Standard `pass` can still _read_
the re-encrypted `.gpg` files (still OpenPGP, still encrypted to keys in its
keyring), but the store is no longer maintained as a `pass` store. A clean break
to gpm's native age format (GPG→age) is a separate, heavier conversion and is
out of scope here.

**Single-user is the cheap degenerate case.** When a store's only recipient is
the converting user, existing secrets are already encrypted to them, so
conversion reduces to seeding `.public-keys/` from their imported key (no
re-encryption of existing entries). The general path handles this and the
multi-recipient case uniformly, which is why the general path is built once
rather than shipping a single-user slice.

**Recipient-shrinkage safety.** Re-encryption must never silently reduce the
recipient set; gopass's behavior of refusing to encrypt to a partial set when a
recipient key cannot be resolved is the compatibility bar.

**Destructiveness and recovery.** Conversion rewrites ciphertext; it is one-way.
The design preserves recoverability in git history (commit before and after the
re-encrypt) and surfaces the multi-recipient consequence up front when teammates
share the store.

## Alternatives considered

- **Single-user-only convert (encrypt-to-self slice).** Rejected as the v1
  shape: it would be thrown away once multi-key stores appear, and it silently
  breaks shared stores by dropping other recipients. Build the general path once,
  on the R094 + R083 foundation.
- **Convert to age instead of gopass-GPG.** A GPG→age re-key is the cleaner
  "leave GPG entirely" path and matches gpm's age-first stance (A001), but it is
  a heavier bulk re-encrypt and severs `pass` readability. The chosen target is
  gopass-GPG because it reuses R083's engine and, for single-recipient stores,
  needs no re-encryption at all; an age target can be a follow-up.
- **Read-write in place (no convert).** Rejected: would require resolving
  arbitrary `pass` recipients' public keys (keyserver / keyring), which A006
  rules out.

## Effort

~medium-large (human ~3–4 days / CC ~40 min): mostly orchestration, but it
inherits its prerequisites' re-encryption cost and must handle bulk atomicity,
progress, conflict/autosync interaction, and gopass-interop tests.

## Depends on / Supersedes

Serves `docs/specs/004-encryption-gpg`. **Blocked on** **R094** (GPG
multi-identity, to decrypt multi-key stores) and **R083** (GPG recipient/keyring
management + re-encryption). Builds on **R093** (read-only `pass`, the read
foundation). Related to gopass's `convert` and `recipients add/remove`.
