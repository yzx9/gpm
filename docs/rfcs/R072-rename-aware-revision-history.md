# Rename-aware secret revision history

**Priority:** P3
**Status:** Draft
**Phase:** Future

## What

A future extension of secret revision history (shipped as the path-bound RFC
R027, now in code) that follows a secret **across renames**: its history would
include versions the file held under a prior name, not only those at its current
path. Decides `docs/specs/NNN-rename/` (the rename feature, not yet built).

## Why

R027's revision history is **path-bound** — a secret's history is the set of
commits that touched its _current_ file path. That is complete today only
because gpm has no rename feature: a file cannot move, so its current path has
always been its path. The moment gpm gains renaming, path-bound history
**silently drops everything before the rename** — and it drops it exactly when
the user is most likely to want it (right after a rename, looking for the value
as it was before). Rename-aware history restores the completeness that makes
R027's recovery and audit value survive a rename.

## Context

R027 is path-bound by deliberate design, matching gopass (whose `history` /
`show --revision` also do not follow renames). The load-bearing reason is the
**path-at-commit view invariant**: viewing a revision reads "this path at that
commit," which is only well-defined when the path actually existed at that
commit. Path-bound listing guarantees that — every commit it returns touched the
exact path being viewed. Rename-awareness must either preserve this invariant or
supersede it: the natural shape is to resolve a rename chain back to the
**historical** path at each commit (the name the file held then), so the view
still reads "the path-as-it-was at that commit."

Two things make this materially harder than the path-bound walk R027 shipped:

- **No libgit2 `--follow`.** `git log --follow <path>` is a git-CLI feature with
  no direct libgit2 equivalent. gpm's revision walk is built on the `git2` crate
  (revwalk + per-commit tree diff), so rename-following means per-commit **rename
  detection across a path that changes at each step** — the "current path" is a
  moving target, recomputed as each rename is detected. This is a different order
  of complexity from the literal pathspec diff R027 uses.
- **It is forward of gopass.** gopass's own revision listing does not follow
  renames, so there is no compatibility anchor to match; the semantics are gpm's
  to define.

The threat model is unchanged: revisions remain content-addressed (a commit
hash is a stable anchor; gpm only appends commits), ciphertext never crosses into
the untrusted layer, and an undecryptable past revision still surfaces as a state
rather than a blob. Rename-awareness is purely about _which_ commits count as
"this secret's," not how a located revision is read.

## Alternatives considered

- **Path-bound forever (the R027 model).** Simplest, and exactly right while
  gpm has no rename. Rejected only once rename exists: it then hides pre-rename
  history at the moment it matters most.
- **Shell out to `git log --follow`.** Rejected: gpm's git layer is libgit2 by
  design (the Termux-binary wall), not the git CLI. R027 walked the path in-tree
  for the same reason.
- **Per-commit rename-detection walk (the likely approach).** Revwalk with
  rename detection enabled in the per-step tree diff, tracking the path as it
  changes; the path-at-commit view resolves through the detected chain. Recorded
  as the direction to evaluate when rename lands; not designed here.

## Effort

~medium-large (human) / ~medium (CC) — but **gated on the rename feature
existing first**, which is itself unbuilt. The rename-detection walk is the
non-trivial part; the locate-then-read reuse from R027 (blob-at-commit, decrypt,
reveal/auto-clear) carries over unchanged.

## Depends on / Supersedes

Deferred until gpm gains a rename feature (`docs/specs/NNN-rename/`, not yet
built) — until then path-bound history is complete and this is moot. Revisits
the path-bound decision in the shipped secret-revisions feature (formerly R027,
now in code); that RFC is deleted on ship, so this RFC restates the path-at-commit
invariant and the rename question at its own altitude rather than relying on it
surviving.
