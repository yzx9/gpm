# Export a past revision's attachment

**Priority:** P3
**Status:** Draft
**Phase:** Future
**Revision:** 1

## What

Export the binary attachment held at a specific past commit of a secret — the
per-revision counterpart to the live attachment export (`export_attachment`).
From the Revisions view of an attachment revision, save that old version's
decoded file to a user-chosen path.

## Why we are NOT doing this (deferred)

This RFC records a **deliberate deferral**, not a backlog item. The shipped
revision feature already surfaces an attachment revision honestly — a
"Binary attachment · {filename}" notice ("gpm doesn't export past attachment
versions — open the entry for the current one") instead of a blank reveal, and
`copy_revision` short-circuits so it never falsely reports "Copied." So the
attachment × revision gap is closed UX-wise without building the export path.
It stays deferred because:

- **Niche on niche.** Attachments are already a small fraction of a password
  store; restoring a specific _past_ version of one is rarer still. The
  recovery story ("I overwrote last week's file and want it back") is uncommon.
- **Beyond gopass.** gopass's `history` / `show --revision` does not export a
  past attachment version — it would only echo the base64. There is no
  compatibility anchor to match, so this would be a gpm-original feature, and
  gpm's stance is to align with gopass and narrow, not extend beyond it here.
- **Recoverability already exists.** The bytes are in git history; a power user
  (the Jordan persona) recovers via `git show <commit>:<path>.age` piped through
  a base64 decode, or via gopass itself. The feature would be a convenience,
  not a capability gap.
- **Scope discipline.** The shipped revisions feature is "read-only view + copy
  of past _text_ secrets." Extending it to binary attachment export pulls in
  attachment-metadata surfacing, a new UI affordance, and a decrypt-from-commit
  → decode → stage → save pipeline — a distinct feature that would dilute that
  clean scope.

## Context (the design space, if revisited)

Most ingredients already exist:

- `Store::get_at_revision(name, commit)` already decrypts a past blob (the
  revision-view path).
- `rustpass::attachment::extract(body)` base64-decodes an attachment body and
  returns its bytes + filename.
- The export pipeline in `export_attachment_core` — single-flight
  (`FileSaveGuard`) → decrypt → decode → stage (`StageGuard`, 0600) → save
  plugin → wipe-on-drop, plus the startup `sweep_attachment_stage` — is
  reusable as-is.

A minimal implementation adds an `export_attachment_revision(entry_path,
commit)` command: `get_at_revision` → `attachment::extract` → the existing
stage + save pipeline (decoded bytes never cross the IPC boundary, identical to
the live export). Frontend: the revision detail sheet's already-shipped
attachment state gains an "Export this version" action. Nothing about the threat
model changes — the stage holds decrypted bytes for the same window the live
export does.

The non-trivial part is none of the above — it is the _reach_. A revision is an
attachment only when the entry's type changed over history (gpm never writes
attachments itself, so this only arises in mixed gpm+gopass or raw-git use, or
by hand-editing the store). Building the full export path for that narrow case
is the cost not justified by the value above.

## Depends on / Revisits

Deferred indefinitely. Revisit if attachment _write_ lands in gpm and a
past-version restore becomes a natural pair, or if real demand surfaces. Until
then the shipped attachment-revision notice + copy short-circuit cover the UX.
Revisits the shipped secret-revisions feature's attachment handling; that
feature's own RFC was deleted on ship, so this one records the deferral at its
own altitude.
