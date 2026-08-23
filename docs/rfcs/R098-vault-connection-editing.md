# Vault connection editing — rotate SSH auth in place, settle URL semantics

**Priority:** P2
**Status:** Draft
**Phase:** Next
**Revision:** 1

## What

Let a configured user edit an existing repository's git connection in place: rotate the SSH private key and its optional passphrase without re-cloning, re-entering the age identity, or resetting anything (PAT rotation already ships), and update the remote URL when it is the same repository reached differently (transport switch, host migration). This RFC also records the line it draws: pointing the vault at a _different_ repository is not an edit — under multi-repository that is remove + re-add.

Serves `docs/specs/005-git-storage` (the repo-connection lifecycle: set up / reconfigure). Supersedes `R004` (deprecated; its PAT slice shipped separately, its remaining SSH slice lives here).

## Why

Rotating credentials is routine security hygiene, and the PAT half is already solved: the PAT page probes a candidate token against the remote read-only and swaps it in only if the probe passes. The SSH half still has no in-place path — the key can be viewed, exported, or cleared, but not replaced; a key rotation today means clear-and-fall-back-to-PAT or a full reset + re-setup (re-enter URL, credential, identity; re-clone the whole store). That is the disproportion R004 existed to remove, and multi-repository does not remove it: remove-vault + re-add narrows the blast radius from the whole app to one vault, but for that vault still deletes the clone (full re-download), the sealed identity, and the authenticity trust-set. The credential is independent of all three — the edit should be two fields. Rotation events also scale with vault count: N vaults across N hosts mean N expiring PATs and N rotatable keys.

The URL question needs a decision recorded because it is now reachable: under multi-repository, the honest expression of "point this vault at a different repository" is remove + re-add (a fresh clone is inherent to a different repository), while "same repository, different URL" (HTTPS → SSH transport switch, host migration) is an in-place edit — history is unchanged, only the transport config moves. No document currently owns either half.

## Context

**The mutation pattern is established and single-field.** Each connection field already has a setter that loads the sealed repo config, mutates only its own field, and saves — the commit-identity, verification-mode, and PAT setters all work this way, and the SSH-clearing setter already treats key + passphrase as one unit. The SSH rotation is the missing sibling: replace the pair atomically after a successful probe. The setup-time save helper must not be reused — it blanks every field it does not know, correct at setup, fatal afterward.

**Probe-then-swap, extended to SSH.** The read-only auth probe (a fetch into a throwaway ref, nothing checked out, cancellable) currently proves a candidate PAT. The probe's underlying primitive already carries SSH credentials — the transport layer authenticates with either — so proving a candidate key + passphrase before saving is a thin surface extension, not new machinery. Probe-then-swap is the property worth keeping: a typo'd or revoked key is rejected up front and the prior credential pair stays intact (atomic swap), so the user is never left with a saved-but-broken credential discovered only at next sync. A credential revoked _after_ a successful probe still surfaces as the next sync's auth error — the same residual window the PAT flow has.

**The UI home is the per-repository settings surface.** Today that is the repository settings page with its connection card (URL shown read-only, active auth method, masked PAT preview, links to the PAT and SSH-key pages); under multi-repository (R080) the same card becomes per-vault. The SSH-key page gains a replace-key affordance (paste, or generate — generation already exists in the setup form), and the URL becomes editable in place. Masking stays at the IPC boundary: a replaced key is never echoed back, only its presence (and its public half, as today).

**Addressing.** The connection commands belong to the command group that still implicitly targets the active repository; R080's explicit-addressing stance (every repository-touching operation carries the repository identifier) applies to them, and the new commands follow it from day one. If this lands before R080 threads the group, the new commands migrate with it — no design change either way.

**Same-repo URL update is a connection edit, not a migration.** Changing how you reach an unchanged repository — HTTPS to SSH on the same host, or the host itself moving — rewrites the URL and usually the credential alongside, then proves the new pair with the same probe. The clone, history, and trust-set are untouched; the next sync proceeds normally. This is the load → mutate → save shape of the credential swap plus recording the new origin, which setup already does without contacting the host.

**Division of labor with R071.** Adding the _first_ remote to a local-only vault is a different transition with its own load-bearing detail (the first publish must be push-only; a pull-first sync errors on the branch that does not exist yet) and stays in R071. This RFC owns edits to an _existing_ connection: rotate credentials, re-point the same repository. Removing the remote entirely (back to local-only) is not in scope — no user pull has surfaced for it, and re-adding later goes through R071 again.

## Alternatives considered

- **Keep R004 as the umbrella.** Rejected — its premise ("the only lever today is Reset All Data") went stale when PAT rotation shipped; its URL and identity slices were deferred elsewhere (multi-repository, R005); the remaining SSH slice plus the URL semantics are cleaner recorded fresh than edited into a Future-phase RFC whose Why no longer holds. R004 is deprecated with a pointer here.
- **remove-vault + re-add as the rotation path.** Rejected — it destroys the local clone (full re-download on mobile), the sealed identity, and the trust-set for a change that touches none of them. Multi-repository narrows the blast radius but not the disproportion.
- **Attempt-and-surface instead of probe-then-swap.** Rejected as the primary path — the probe is already the shipped UX for PAT and gives the atomic-swap property (a bad credential is never saved). Surface-the-error-on-next-sync remains the fallback for a credential revoked after a successful probe.
- **In-place URL edit for a different repository.** Rejected — a different URL is a different repository; pretending otherwise drags in a re-clone decision tree and a mid-migration config state for a case the add flow already expresses cleanly. The exception is the same-repo URL change, which stays an edit because history is unchanged.
- **A general "edit any repo-config field" control.** Rejected (carried over from R004, still true) — every other field already has its own focused setter and surface; only the connection fields lack one.

## Effort

Small. Backend: one SSH pair setter on the sealed-config mutation pattern, the probe surface extension, and — for the URL slice — recording a new origin on the same pattern. Frontend: a replace-key affordance on the SSH-key page and an editable URL on the connection card. The load-bearing parts — single-field sealed-config mutation, the cancellable read-only probe, IPC masking — all exist. (human: ~0.5–1 day / CC: ~1 session)

## Depends on / Supersedes

- Supersedes `R004-reconfiguration-flow` (deprecated): its PAT slice shipped with the PAT management page; the SSH slice and the URL semantics live here.
- Serves `005-git-storage` (repo-connection lifecycle: reconfigure).
- Builds on `R080-multi-repository` — the per-vault connection card and explicit repository addressing this surface lives in; the SSH slice can land before R080's management flow and be adopted by it.
- Coordinates with `R071-add-remote-to-local-only-store` — the no-remote → has-remote edge of the same card; R071 keeps that transition and its push-only first publish.
- Independent of `R005` (identity) — connection fields touch no identity work.
