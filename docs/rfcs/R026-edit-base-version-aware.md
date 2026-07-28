# Edit: base-version-aware (no silent clobber)

**Priority:** P2
**Status:** Draft
**Phase:** Future

## What

Make secret edit refuse to silently overwrite a teammate's newer version of the
same entry. Today an edit built on a stale read can fast-forward over a newer
remote change and return `Written` with no conflict surfaced. This RFC captures
the deferred fix: base-version-aware edit.

## Why

Credential rotation is the most common reason to edit, and a common multi-device
flow is "open the entry on one device, rotate it on another." Today that loses
data silently: device A opens `servers/db` (a snapshot at read time); device B
rotates the password and syncs; device A saves an edit built on the stale
snapshot. Because A had no local commit since its last sync, A's save
fast-forwards over B's rotation and returns success — B's newer credential is
gone from the tip (recoverable only via git history, out of band). The mirror
case is resurrection: if B deletes the entry instead of editing it, A's save
brings it back. In a password manager silent credential loss (or silent
un-deletion) is unacceptable. The write primitive's conflict detection only
fires on push rejection (which needs local divergence), so it can't see either
case on its own; edit must carry a base version and check it.

## Context

Capture the entry's version when the user opens it (its blob oid at read time),
and on save — after the best-effort sync — refuse to write if the entry's current
version differs from that base, surfacing the same write-conflict outcome the
create path already resolves. This reuses the "expected oid, refuse if the remote
moved since" pattern already in the sync-divergence resolution, applied per-entry
to a write rather than per-repo to a pull. The conflict then resolves through the
existing write-conflict path unchanged: "keep mine" intentionally overwrites (now
an informed choice, not silent), "keep theirs" adopts the remote, "cancel" backs
out. The base oid is not secret, so it can cross IPC without a stash; only the
edited plaintext is stashed on conflict, exactly as today.

## Alternatives considered

- **Content compare at save.** Re-decrypt the entry post-sync and compare to the
  base plaintext the user edited from. Rejected: round-trips the current plaintext
  across IPC a second time (the conflict stash exists precisely to avoid
  re-sending plaintext) and re-decrypts on every save.
- **Force a local divergence before every edit.** Rejected: artificial, and would
  surface conflicts even when the remote never touched the entry.
- **Leave edit base-version-unaware forever.** Rejected as the long-term answer:
  acceptable as a documented first cut (see the known limitation in
  `0020-edit-secrets.md`), but rotation-loses-data is the exact scenario edit
  exists to make safe.

## Effort

~0.5 day (human) / ~20 min (CC). Adds a base-oid field to the read path, a
base-oid param plus a per-entry version check on the write path, and a test that
flips from pinning the clobber to asserting the conflict.

## Depends on / Supersedes

Builds on `0020-edit-secrets.md` (edit shipped base-version-unaware; this closes
its known limitation).
