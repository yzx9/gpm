# Edit existing secrets

**Priority:** P1
**Status:** Implemented
**Phase:** Next

## What

Add the ability to edit an existing secret's content in place — change a rotated
password, fix a note, update metadata — without deleting and recreating the entry.

## Why

The store can create and read but not mutate. A password client that can't edit a
rotated credential forces a delete-and-recreate (losing the entry's path
continuity) or simply isn't usable for credential rotation, which is the single
most common reason to open a password manager. Edit is also an operation where a
remote divergence is likely — another device changed the same credential — so it
must compose with the existing decrypt-aware conflict model rather than silently
clobber a teammate's newer data.

## Context

Edit is the store's raw write primitive applied to an entry the user first
decrypted and reviewed. That primitive already overwrites in place; the only new
behavior is gating on existence, so a typo in an edit form can't silently create
a stray entry. A matching creation template is NOT re-applied on edit — templates
shape new secrets, and silently re-templating an entry the user hand-edited would
clobber their edits with a freshly rendered template. So edit operates on the raw
decrypted body: the first line is the password, the rest is notes, exactly as the
user saw them. Because edit goes through the same write primitive as creation, the
existing conflict machinery covers it unchanged: if the remote advanced and holds
a different version of the same entry, the write surfaces a decrypt-aware conflict
for the user to resolve rather than silently overwriting newer data.

## Alternatives considered

- **Re-apply the template on edit.** Rejected: would silently overwrite hand-edits.
  Templates shape creation, not mutation.
- **Delete-and-recreate instead of edit.** Rejected: loses the entry's name/path
  continuity, is more steps, and still has the same conflict exposure.
- **Don't gate on existence.** Rejected: a typo'd edit name silently creating a new
  entry is a footgun; the gate is free because the decrypt-and-show flow already
  requires the entry to exist.

## Effort

~0.5 day (human) / ~15 min (CC). Edit is almost entirely surface (an edit form)
over an existing write primitive.

## Depends on / Supersedes

None. Composes with the existing write/conflict path and with
`0016-recipients-pinning.md` (edit encrypts to the pinned recipients set via the
shared write primitive).

## Revision — implemented approach

Implemented as an existence-gated raw write: `Store::update` checks the entry
exists (so a typo'd name can't create a stray entry) then delegates to the raw
write primitive, which syncs, encrypts, commits, and pushes with no template
re-applied (templates shape new secrets, not mutations). Edit reuses the existing
write-conflict machinery and stash unchanged — on a same-name divergence it
surfaces the same conflict outcome the create flow already resolves.

The conflict UI that lived inline in the create page was extracted into a shared
modal component (the create page and the entry detail page both render it now),
since the conflict resolution UX — including the security-critical
"force-overwrite only after explicit confirmation" gate — is identical for create
and edit.

### Known limitations: fast-forward clobber + resurrection

The write primitive's conflict detection fires only on push rejection, which
requires local divergence. An edit is built on a prior read with no intervening
local commit, so when another device changes the same entry and pushes before the
user saves, the pre-write sync fast-forwards over the newer version, the edit
commits on top, and the push fast-forwards — returning success and silently
overwriting the teammate's change (recoverable via git history).

The same root cause also resurrects a deleted entry: if another device deletes
the entry and the local (without syncing) edits it, the existence gate passes on
the local copy, `set`'s sync fast-forwards to the deletion, and the write
re-creates the file — so the edit silently brings back what the teammate removed.

Both are facets of the base-version-unaware write (the gate checks local state,
then `set` syncs). They are the ways this RFC's "compose with the conflict model
rather than silently clobber" promise is not yet met; the base-version-aware fix
is captured in `0022-edit-base-version-aware.md`, and both behaviors are pinned
by regression tests so they can't drift silently.
