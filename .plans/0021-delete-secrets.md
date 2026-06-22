# Delete secrets

**Priority:** P1
**Status:** Implemented
**Phase:** Next

## Revision — implemented approach

The sections below describe the _originally drafted_ design (a delete-specific
three-way conflict model mirroring create's). On implementation the design was
**simplified** at the user's direction: delete does not resolve conflicts inline
at all. It mirrors the write path's happy path (best-effort sync → existence gate
→ remove → commit → push) and **defers all conflict handling to the existing
sync/divergence flow**: if the push is rejected (remote diverged), the local is
rolled back to the pre-delete state and the caller is told to sync; there is no
"force delete." A non-rejection push failure (offline / auth) propagates with the
local delete commit retained, so it syncs later — mirroring how create handles an
offline write.

Rationale: a deletion has no "our version" to keep, so the four-way
decrypt-aware resolution vocabulary adds surface without clear payoff. The safe
default "if in doubt, don't destroy" holds by rolling back on rejection. The
remote-inspect step and the conflict modal were dropped; `remote_decryptable`
plays no role in delete. Deleted entries remain in git history (no
graveyard/tombstone), but gpm exposes no in-app restore — recovery is an
out-of-band git operation.

## What

Add the ability to remove a secret from the store — decommissioned credentials,
test entries, duplicates — committing and syncing the removal like any other
change.

## Why

Create-only stores accumulate cruft. Without delete, dead entries clutter search
and erode trust in the list, and there is no way to remove a compromised or leaked
credential short of an out-of-band git operation. Delete is also where a silent
remote divergence would be most harmful: another device removed or changed the same
entry, and blindly pushing a removal would discard a teammate's newer version. So
delete must reuse the write path's sync→commit→push→on-rejection-conflict
structure and its decrypt-aware "you can't read what you'd be destroying" guard.

## Context

Delete mirrors the write path: pre-sync, capture the pre-delete state, remove the
entry, commit the removal, push, and on a rejected push roll back and decide
whether to replay the deletion (the remote moved on unrelated files) or surface a
conflict. The conflict model is narrower than creation's: a deletion has no "our
version" to keep — no plaintext is involved — so the resolution vocabulary
collapses to delete-anyway (force the removal onto the remote; destructive when the
remote entry is undecryptable, since we'd be destroying data we can't read),
keep-the-remote-entry (abort the delete), or cancel (leave the store as-is). There
is no separate "force" tier because there is nothing extra to confirm —
delete-anyway is already the explicit, destructive option, and the user's confirm
dialog is the gate. The remote entry's decryptability still matters: it decides
whether the conflict modal can offer an inspect-the-remote step before the user
commits to destroying it.

Recovery model: gpm does NOT keep a gopass-style "graveyard" tombstone. Every change
is committed, so git history preserves the deleted ciphertext blob — an accidental
delete is recoverable out-of-band by checking out the old blob. A first-class in-app
undelete (walking history) is deferred; the recovery path exists, it is simply not
yet surfaced in the UI. This avoids doubling the store's file vocabulary and the
list/search complexity of hiding tombstones.

## Alternatives considered

- **Graveyard / tombstone for delete.** Rejected for now: doubles the store's file
  vocabulary, complicates list/search (must hide tombstones), and git history
  already preserves the deleted blob. A future undelete UI can walk history without
  changing the on-disk format.
- **Reuse the four-way creation conflict choice for delete.** Rejected: the
  "force keep mine with extra confirmation" step has no meaning for a deletion
  (there's no second plaintext to guard), so it would be a confusing no-op. A
  three-way delete-specific choice is cleaner.
- **Server-side refusal to delete an undecryptable remote.** Rejected: unlike
  creation (where the non-force option is the safe default), there is no
  non-destructive default delete action to protect — delete-anyway is inherently
  the confirmed destructive action, and refusing it would dead-end the user. The
  frontend confirm dialog is the gate.

## Effort

~1.5 days (human) / ~30 min (CC). Delete is a new write capability, but it mirrors
the existing write path's structure and reuses most of its machinery.

## Depends on / Supersedes

None. Delete touches no recipients, so it is independent of
`0016-recipients-pinning.md`.
