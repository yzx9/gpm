# Multi-identity support for GPG stores + `.gpg-id`

**Priority:** P3
**Status:** Draft
**Phase:** Future

## What

The GPG/OpenPGP counterpart of R005: hold multiple OpenPGP secret keys as
decryption identities, and use each store's `.gpg-id` — including the
per-sub-directory `.gpg-id` (R078) — to pick the right identity for each secret,
instead of trying one key against every file. Closes the asymmetry whereby age
and SSH stores (R005) have a multi-identity story and the GPG backend does not.
Serves `docs/specs/006-identities` and `docs/specs/004-encryption-gpg`.

## Why

R005 gives age/SSH stores multi-identity decryption; the GPG backend has no
equivalent. A GPG store whose sub-directories list different recipients — a team
sub-store layout, or a key-rotation window with old and new keys both live —
needs more than one secret key to decrypt in full. Today gpm holds a single
identity, so only the sub-trees encrypted to that one key are readable. This RFC
gives every crypto backend the same multi-identity story, and it is the
foundation for the general convert path (R095), which must decrypt every secret
before re-encrypting.

## Context

**Mirror of R005, for GPG.** Replace the single GPG identity with a labeled
collection; when listing entries, parse each relevant `.gpg-id` to learn which
recipients a secret is encrypted to and surface which identities are missing;
when decrypting, select the matching GPG identity, falling back to trying all
identities for back-compat with stores gpm cannot fully classify.

**Recipient source differs from age.** `.age-recipients` lists public keys
directly; `.gpg-id` lists identifiers (key IDs / fingerprints) that gpm resolves
to its own _imported identities_ by fingerprint / key-ID match — the same
resolution R093 uses for the membership gate. gpm does not resolve `.gpg-id`
against a system keyring (A006); only the identities the user has imported are
visible.

**Per-sub-directory recipients.** `pass` and gopass both allow a `.gpg-id` per
sub-directory, scoping different recipients to different sub-trees (R078).
Multi-identity decryption must respect the nearest `.gpg-id` for each secret.

**Overwrite-safety gate.** As with R005, once a store can hold entries a given
identity set cannot decrypt, a "keep mine" overwrite could destroy unreadable
ciphertext; the overwrite-safety gate R005 defers applies here for GPG as well.

**Identity cache/lifecycle.** The unlocked-identity cache and lock lifecycle
(R042) are backend-agnostic; this RFC consumes whatever shape R042 eventually
settles on rather than introducing a GPG-specific cache.

## Alternatives considered

- **Extend R005 to cover GPG in one unified multi-identity RFC.** Rejected: the
  two backends' recipient formats and identity types differ enough that a single
  RFC obscures both. Keep R005 (age/SSH) and this RFC (GPG) as parallel,
  symmetric records, matching how the crypto backends themselves are tracked.
- **Single GPG identity forever; rely on convert.** Rejected: reading a
  multi-key store _without_ migrating it is a legitimate standing need, not just
  a convert stepping-stone (a user may want to keep using `pass`/gopass on the
  desktop and only read from gpm).

## Effort

~1–2 days (human) / ~45 min (CC) — comparable to R005, plus per-sub-directory
`.gpg-id` resolution and a GPG identity import/management UI.

## Depends on / Supersedes

Serves `docs/specs/006-identities` and `docs/specs/004-encryption-gpg`.
Symmetric counterpart to **R005** (age/SSH multi-identity). Relates to **R078**
(per-sub-directory `.gpg-id`) and **R042** (identity cache/lifecycle).
Prerequisite of **R095** (convert).
