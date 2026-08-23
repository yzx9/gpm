# GPG key generation

**Priority:** P3
**Status:** Blocked
**Phase:** Future
**Revision:** 1

## What

Generate a new GPG/OpenPGP keypair inside gpm, so a user can start a GPG store without importing a pre-existing key. **Blocked** — wanted and analyzed, but set aside on a cost/value gate, not a technical one. Recorded here so the analysis survives and is easy to find later. Serves `docs/specs/004-encryption-gpg`.

## Why Blocked

Three reasons, all product/cost rather than capability (rpgp can generate keys):

1. **GPG key management is complex, and reimplementing a credible keygen + key-management flow on Android is high-cost.** GPG keys carry primary/subkey structure, capability flags, expiry, and revocation — a surface gopass itself only thinly wraps over system `gpg`. Building and maintaining that in-app is a large, ongoing burden for a path most users do not need.
2. **Onboarding cost is not worth it for this project.** Generating a key forces gpm to introduce GPG concepts (subkeys, capabilities, expiry) to users who have never touched them — a poor fit for an app that otherwise aims for a simple, modern experience.
3. **For a fresh start, age is the better recommendation.** gpm's new stores are age-only by default precisely because age avoids this complexity; a user starting from scratch should be steered to age, not GPG keygen. GPG's value is for users who already have a GPG store — and they already have a key to import (R082).

Marking this `Blocked` (rather than `Deprecated`) keeps it discoverable for reassessment: if GPG adoption among gpm users grows, or a partner use case demands in-app keygen, the analysis is here.

## Context

gopass keygen defaults to an RSA-2048 keypair via system `gpg`; a gpm keygen would use rpgp and could modernize to Curve25519, documenting the divergence from gopass's default (the on-disk format is algorithm-independent). The produced secret key would be stored S2K-passphrase-locked and AEAD-sealed at rest, and its recipient would seed a fresh store — overlapping R082's seeding. The technical path is well-understood; the blocker is the product decision above.

## Alternatives considered

- **Import-only create (chosen for now — R082).** A user creating a GPG store already has a key; importing it covers the realistic path without the keygen / key-management burden.
- **Recommend age for new stores (the redirect).** For a user with no existing key, point them at an age store — simpler, and gpm's default. This is the expected outcome for the "fresh start" user.
- **Wrap system `gpg` for keygen on desktop only.** Rejected: it splits the create path across platforms and still cannot run on Android (A006).

## Effort

~medium for keygen itself; the real cost is the large UX / concept-onboarding surface, which is the actual blocker.

## Depends on / Supersedes

Depends on A006 (rpgp). Serves `docs/specs/004-encryption-gpg`. Relates to R082 (import-only create, the chosen alternative).
