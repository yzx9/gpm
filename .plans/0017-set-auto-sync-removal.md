# Plan: Remove `Store::set`'s pre-sync? (deferred)

**Priority:** P3
**Status:** Deprecated
**Phase:** Future

## Context

`Store::set` does `self.sync().await?` (pull) before writing — gopass _PushPull_
alignment (pull → write → push). Plan 0012 (pull-sync divergence) changed
`sync()` to return `SyncOutcome`, surfacing a `Diverged` variant. That forced a
decision: what does `set` do when its pre-sync sees divergence?

- **A** — `set` refuses (`PullFfFailed`).
- **B** — `set` ignores `Diverged`, proceeds (best-effort pull).
- **remove** — drop the pre-sync entirely; `set` writes, pushes, and lets the
  push-rejection path handle everything.

## The question

The pre-sync is a **speculative** check: it predicts whether the upcoming push
will be rejected. The **authoritative** check is the push result itself — and
the write path already handles push rejection (`set` rollback → replay on other
files | same-name `Conflict`). "When to sync" is also arguably an
application-layer policy (the app already pulls via the pull UI / 0012's modal),
not something `set` should impose. So is the pre-sync redundant?

## Analysis of removing it

Structurally clean; dissolves the divergence-in-`set` question entirely. Two
implications:

1. **`create` on a remote-existing same-name entry.** Today the pre-sync pulls
   it in and the write silently updates it (`Written`, upsert). Without the
   pre-sync, the push is rejected → `fetch_remote_blob` finds it → **`Conflict`**
   (keep mine / keep existing / view). Safer for a password _create_ flow (no
   silent overwrite of a remote secret the user hadn't synced), but diverges
   from gopass upsert semantics.

2. **Enforce verification (marginal).** The pre-sync's `pull_verified`
   verifies-and-ffs only in the clean "remote advanced, all signed" case,
   avoiding `fast_forward_to_remote` there. But once Enforce blocks or the repo
   diverges, `set` already falls through to push-rejection →
   `fast_forward_to_remote` (no verify — codex ④). So removing the pre-sync only
   marginally widens the existing ④ bypass; it is not a new class of problem.

## Decision (this branch)

**Keep the pre-sync; go with B.** `set` ignores `SyncOutcome::Diverged`
(best-effort pull, proceeds to write). Rationale: minimal change, preserves
current upsert semantics + gopass alignment, and the `write_conflict` test suite
passes unchanged. Divergence resolution lives in the **pull** path (0012's
modal), not in `set`.

## When to revisit

Remove the pre-sync if/when we want:

- `create` to surface a `Conflict` (not silently update) when the name exists
  remotely-but-not-locally, **and/or**
- to simplify the write path and let the push result be the sole authority.

Revisit **together with codex ④** (write-path Enforce bypass via
`fast_forward_to_remote`): they share the `fast_forward_to_remote` hard-reset
and are best fixed in one pass (verify-before-adopt in the write path).

## Tests (if removed later)

- `create` on remote-existing same-name → `Conflict` (pins the new behavior).
- All existing `write_conflict` tests continue to pass (they go through
  push-rejection regardless).
