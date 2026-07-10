# Backend-owned provenance verification

**Priority:** P3
**Status:** Draft
**Phase:** Future

## What

Make repository authenticity (provenance) verification a responsibility of the
storage backend rather than the store facade. Today the store assumes
authenticity means git commit-signature verification and reaches past the
backend into the revision-control layer to perform it. This RFC relocates that
behind a single backend seam: the store asks the backend "is this repository
authentic, and on what basis?" and the backend answers from its own model —
signature verification for the git backend, an honest "no provenance model" for
the no-revision-control backends, and whatever trust signal a future backend
may carry.

## Why

Authenticity is coupled to git in two places at once: it is *implemented* only
for git (commit signatures), and it is *invoked* from the store as if git is
the only possibility. The first pluggable-storage backends (RFC 0046's
local-only and cloud-folder backends) have no revision control and therefore no
commit signatures, so for them authenticity is not applicable — which forces
the store to explicitly gate the git verification path off for them. That
gating is correct and ships first. But it hard-codes the deeper assumption that
"authenticity means commit signatures." A future backend that carries its own
trust signal — a signed manifest, a content-addressed root, a provider
attestation — has nowhere to declare it; the store would again assume git, or
nothing.

Moving the provenance question behind the backend trait makes each backend own
its own answer. The store stops being the place where "authenticity means
signatures" is baked in, and a backend with a real trust signal has a place to
express it without re-touching the store.

## Context

**The reference model — gopass treats authenticity as a git property.** gopass's
signature verification is a function of its git revision-control backend; the
no-RCS `fs` backend has no equivalent and makes no claim to it. gpm inherits
this: provenance is meaningful only where there is a signed history. This RFC
does not invent a new provenance model; it relocates the existing one so its
*location* matches gopass's conceptual split — it is the backend's concern, not
the store's.

**Why a backend seam, not a store policy.** A store-level authenticity policy
that branches on backend kind re-creates the current coupling in a new shape:
the store would still enumerate backends and decide per kind, and a new backend
would still require a store change to be recognized. The dependency-inverted
shape — the backend declares its model, the store asks — keeps the store
backend-agnostic and leaves room for a backend to introduce a trust signal the
store does not know about. This mirrors the storage abstraction's own lesson:
the trait is shaped by what a second backend needs, and authenticity is part of
that surface.

**What "no provenance" means, honestly.** The local-only and cloud-folder
backends cannot make an authenticity claim. The local-only store's integrity is
whatever the user's external sync tool provides; the cloud-folder store's is
the cloud provider's conflict model. Neither is a cryptographic attestation of
authorship. Declaring "no provenance model" is the honest answer for them, not a
gap to paper over. The seam's value is not that these backends gain a feature;
it is that a *future* backend with a real trust signal has a place to put it,
and that the store stops assuming the only such signal is a commit signature.

**The near-term gating is the prerequisite, not the alternative.** Until this
RFC lands, the store verifies signatures only when the backend carries signed
history (a git backend) and skips verification otherwise, surfaced to the user
as a git-only property. That gating is the correct short-term behavior and ships
first; this RFC then replaces the gate with a backend-declared model, so "no
provenance" becomes a default backend answer rather than a special case the
store has to remember.

**Threat-model impact — none for contents; honesty about metadata.** Secret
contents stay encrypted exactly as today; this moves only the provenance
*check*, not the encryption. What changes is that a user on a no-provenance
backend is told, explicitly, that authorship is not attested — the same honest
position a gopass `fs` user is already in — rather than silently inheriting a
signature check that does not exist for their backend.

## Alternatives considered

1. **Keep authenticity as a store policy that branches on backend kind.**
   Rejected: it re-creates the current store↔git coupling in a new form (the
   store enumerates backends to decide), and leaves no seam for a future backend
   to declare a trust signal the store did not anticipate without a store
   change.

2. **Leave the near-term gate in place permanently.** Rejected as the long-term
   answer: the gate is a correct short-term fix, but it hard-codes "authenticity
   means git signatures" as the store's only model. A backend that later carries
   its own trust signal would have nowhere to express it.

3. **Define a rich, backend-generic provenance vocabulary now (signatures,
   manifests, attestations).** Rejected: it would be designed from a single
   real implementation (git signatures) and guessed for the rest — the same
   single-implementation trap that shaped the storage and crypto abstractions.
   The seam should start minimal (a backend declares whether and how it attests
   provenance) and be reshaped when a second backend actually has a model to
   declare.

## Effort

Small to medium. ~1–2 days human / ~15 min CC. One new backend seam with a git
implementation (relocating today's signature verification behind it) and a
no-provenance default for the no-RCS backends, plus routing the store's
verification through the seam. The verification logic itself already exists;
this is relocation plus a seam, not new cryptography. Low risk
(behavior-preserving for git; honest no-op elsewhere), forward-looking value (a
place for a future backend's trust model).

## Depends on / Supersedes

Depends on `0046-pluggable-fs-storage-backend.md` — this RFC relocates a
responsibility onto the backend seam that 0046 introduces. Builds on the
near-term gating (verify only where the backend carries signed history) as the
prerequisite that keeps behavior correct until the relocation ships. Relates to
the existing commit-signature verification work (SSH-signed and GPG/OpenPGP-
signed commits), which the relocation moves behind the seam.
