# GPG sub-directory recipients (per-sub-tree `.gpg-id`)

**Priority:** P3
**Status:** Draft
**Phase:** Future

## What

Support gopass's per-sub-directory recipient partitioning for GPG stores: a
sub-directory may carry its own `.gpg-id`, and secrets within it are encrypted to
that sub-tree's recipients rather than the root set. gpm's GPG backend resolves
recipients from the root `.gpg-id` only; this RFC records the gap and defers full
support to a later phase. Serves `docs/specs/004-encryption-gpg`.

## Why

Today gpm resolves GPG recipients from a single index at the repo root. For a
store that uses sub-tree partitioning (a team repo where, say, `team-a/` carries
its own `.gpg-id`), reads still work — OpenPGP decryption needs only the user's
key to match the recipients a given secret was actually encrypted to, which is
independent of any `.gpg-id` — but writes mis-route. A new secret written anywhere
in the store is encrypted to the root recipient set, not the enclosing
sub-tree's, which can expose it to root-level recipients who should not see it,
or make it undecryptable by the sub-tree's members. That silent write-time
mis-routing is a correctness and security gap, not merely a missing convenience.

## Context

gopass resolves recipients per secret by walking up from the secret's directory
to the nearest `.gpg-id`. gpm's GPG backend resolves one recipients index at the
repo root, established in the GPG backend (spec 004). The read path is unaffected by the difference;
only the encrypt path is.

Interim v1 stance: the limitation is documented, with no detection or special
handling. A write into a sub-tree-partitioned store therefore silently encrypts
to the root set — the security caveat above applies until full support lands.
Full support means resolving the nearest enclosing `.gpg-id` per write path,
mirroring gopass, including per-sub-tree recipient public-key lookup.

## Alternatives considered

1. **Full per-sub-tree resolution now (gopass parity).** Rejected for v1:
   non-trivial (walk-up resolution per write plus per-sub-tree public-key
   lookup), serves a niche — most gopass-GPG stores are root-only — and v1's
   focus is read plus single-level write. Deferred by way of this RFC.

2. **Detect sub-tree partitioning at open and mark those stores read-only in
   v1.** The honest-error mitigation if sub-tree stores must be handled before
   full support: reads fully work, writes blocked with a clear message. Rejected
   for v1 as well — it adds detection machinery for a niche case now; it is the
   first option to reach for if the silent-mis-routing risk bites a real user
   before full support lands.

3. **Detect and warn on each write.** Noisier than read-only and still permits
   the mis-route when the warning is dismissed. Inferior to (2) if any
   mitigation is wanted.

## Effort

~medium for full support: walk-up `.gpg-id` resolution per write path,
per-sub-tree recipient public-key lookup, and tests against a partitioned
fixture. ~small for the read-only mitigation (2), should it be wanted first.

## Depends on / Supersedes

Relates to the shipped GPG backend (`docs/specs/004-encryption-gpg`, which
established the root-only recipient resolution).
