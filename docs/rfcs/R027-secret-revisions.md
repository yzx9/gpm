# Secret revision history

**Priority:** P2
**Status:** Draft
**Phase:** Next

## What

gpm already stores every secret change as a git commit but exposes none of that
history in-app. Add the ability to list the revisions of a single secret and to
view — read-only, with copy — any past version. A per-secret counterpart to
gopass's `revisions` and `show --revision`.

## Why

Every edit, create, and delete is already versioned in git, so the data exists —
it is simply not surfaced. Surfacing it answers two real needs:

- **Recovery.** An accidental overwrite on save destroys the previous value with
  no in-app way back; today the only recovery is raw git surgery on the repo.
- **Audit.** "What did this secret hold last week, and who changed it?" is
  unanswerable today.

Both are met purely by _showing_ history; neither requires writing back.

## Context

Two operations make up the feature: (1) list a secret's revisions — the commits
that changed it, with author, date, and message; and (2) view one revision —
decrypt and reveal that past value. Listing needs no decryption at all; it is
pure history, so it always succeeds, is cheap, and — crucially — a revision's
metadata is available even when its content is not. Three decisions frame the
shape:

**1. Path-bound history, not rename-following.** A secret's history is the set
of commits that touched its file path. This matches gopass, whose revision
listing does not follow renames. Following renames would do two unwanted things:
couple listing to rename detection, and — more importantly — break the view
path, which reads "this path at that commit." That read is only well-defined
when the path actually existed at that commit, exactly what path-bound listing
guarantees. gpm also has no rename feature, so there is no current need; the
moment gpm gains renaming, rename-aware history becomes a real question and can
be revisited.

**2. Graceful degradation when a revision can't be decrypted.** A past revision
may have been encrypted to a recipient set the current identity isn't part of —
key rotation, an identity change, or a shared repo where a teammate encrypted a
revision. Such revisions still appear in the listing (metadata needs no key).
Viewing one returns a distinct "undecryptable" state rather than failing — and,
critically, rather than surfacing the ciphertext. Decryption uses the current
identity only: gpm is single-identity, so there is no historical-recipient
notion to reach for, and inventing one would contradict that model. This mirrors
how gpm already treats a remote secret it can't decrypt (it reports the
situation, never the blob) and keeps the threat model intact — ciphertext never
crosses into the untrusted layer.

**3. Read-only view + copy in the first cut; no restore/rollback.** Restoring an
old value is a write — re-encrypt to the current recipients, commit, push — and
inherits the whole write-path surface: base-version awareness, push rejection,
conflict resolution. That is a separate, larger piece, and it is not needed to
deliver the recovery and audit value, which only requires _seeing_ the old value
(and copying it, gpm's primary operation). Defer restore.

Viewing a revision decrypts, exactly as showing the current secret does, so the
existing identity-unlock path applies unchanged — no new auth surface — and the
revealed past value rides the same short-lived reveal / auto-clear / wipe-on-drop
contract the current password already uses.

## Alternatives considered

- **Rename-following history (a `git log --follow` equivalent).** Rejected for
  now: adds rename-detection complexity, breaks the path-at-commit view
  invariant, and gpm has no rename feature. Recorded as future work.
- **Hard-fail on an undecryptable view (gopass's literal behavior).** Rejected
  for gpm: hostile on a mobile client, and inconsistent with the graceful
  "can't-decrypt-this-remote-secret" handling gpm already established. The two
  should behave the same way.
- **Precompute a decryptability badge for every revision in the listing.**
  Rejected: it would turn a cheap, key-free listing into N decryption attempts.
  Decryptability is decided only when a revision is actually opened. A lazy
  badge is a possible follow-on.
- **Restore/rollback as part of the first cut.** Rejected — see decision 3; it
  drags in the entire write path for value the read-only view already delivers.
- **Whole-store commit history instead of per-secret.** gpm already has the
  commit-signature authenticity screen (per-commit, across all secrets); this is
  intentionally per-secret and orthogonal to it.

## Effort

~medium (human) / ~medium (CC). Backend history walk + revision view, two app
commands, one frontend page, and tests — reusing the existing blob-at-commit,
commit-walk, decryptability-probe, decrypt, and reveal/auto-clear primitives.

## Depends on / Supersedes

None. Builds on existing commit-walk and reveal infrastructure.
