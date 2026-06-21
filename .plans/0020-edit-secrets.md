# Edit existing secrets

**Priority:** P1
**Status:** Draft
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
