# Export a repository via Git bundle

**Priority:** P2
**Status:** Draft
**Phase:** Next

## What

Let a user export an entire repository to a portable **Git bundle**. (The symmetric
counterpart — re-importing a bundle as a new repository — is analyzed and **deferred to
R087**; it earns its place only once a non-git backend exists.) A bundle _is_ the
repository — its full history of encrypted secrets —
in a single portable file that can be handed to another device, kept as a backup, or opened
in desktop gopass. Creating a bundle (and cloning from one) is a Git operation that **never
decrypts**, so export and import do not require the vault to be unlocked, and the bundle's
secrets stay encrypted. The bundle exposes the same metadata (entry paths, structure, commit
messages) that the git remote already exposes — no new leak surface.

Serves the repository-lifecycle concerns of `docs/specs/005-git-storage` (a repository is a
git repo; the clone + identity-setup flow), and the multi-repository work in
`docs/specs/009-multi-repository`, where import becomes a third "add vault" source. There is
no separate spec for this feature; it is small enough that the requirements live here.

## Why

Today a repository's data can leave the device only through its git remote. That fails three
real cases:

1. A **local-only vault** (no remote) — the common mobile-first-newcomer case — has no
   off-device egress at all; its only survival copy lives on the phone.
2. **Device-to-device transfer** without exposing or trusting a third-party host.
3. **Graduation to desktop gopass** without first setting up a remote.

Export gives every vault a portable, encrypted artifact; **import closes the loop** so the
artifact is restorable. The connection to the multi-repository "remove vault" action is
direct: export is the safe-egress path that makes removing a local-only vault acceptable
(the removal PRD confirms a local-only vault is permanent loss; export is the way out first).

## Context

**What a bundle is, at the right altitude.** A Git bundle is a single file containing a
repository's object database — commits, trees, and the encrypted secret blobs — as a
packfile, plus the refs. Because gpm stores each secret as an encrypted file inside a normal
git repository, a bundle of that repository is simply the encrypted repository, made
portable. It carries **full history** (revisions are a shipped capability), which matters: a
snapshot would discard the audit trail that is a core reason to keep a _repository_ rather
than a flat export.

**No unlock, by construction.** Bundling packs git objects; it does not decrypt secret
content. So export works under App Lock and never places the decrypted identity or any secret
text in memory. Import — cloning from a bundle — is symmetric: a git operation, no decryption,
followed by the normal identity-setup step, because the bundle's secrets are encrypted to the
original recipients and the importing user must supply an identity that matches one of them.

**Leak surface equals the remote's.** The bundle's secrets are encrypted; what is visible is
metadata — entry paths, directory structure, commit messages — exactly what anyone with read
access to the git remote can already see. A raw bundle therefore adds **no exposure beyond
what the remote model already assumes**. This is why v1 ships the bundle raw (unwrapped):
wrapping the whole bundle in an additional passphrase introduces a _new_ secret that can be
lost independently of the age identity — a "forgot passphrase, lost the backup" footgun that
is especially bad for a survival copy. A raw bundle's recoverability depends only on the age
identity the user already manages. A passphrase-wrapped variant remains a possible future
enhancement for transmission through untrusted channels, with that trade-off called out
rather than made the default.

**Where the bundle goes.** On Android the user picks a destination through the system
save-file picker; on desktop through the standard save dialog. Import picks a source file the
same way. This is the same surface already used for attachment export, so no new platform
plumbing is needed.

**Per-vault, and independent of multi-repository.** Export acts on one repository — the
active one. It does **not** depend on the multi-repository feature and can ship before it:
with a single repository, export is already useful (local-only backup, graduation), and
import is an alternate first-run setup source. When multi-repository lands, import becomes
the third "add vault" source — clone-from-URL / create-new / import-from-bundle — reusing the
same clone + identity-setup flow with the bundle as the source.

## Alternatives considered

- **Encrypted-portable-secrets backup (re-encrypt the secrets under a user-supplied key)
  instead of bundling the repository.** Rejected as the primary shape — it discards git
  history and repository metadata (recipients, authenticity trust-set, commit lineage), so it
  is not "export the repository," and it is not gopass-compatible. It could be a separate
  future feature for a secrets-only portable backup.
- **Config + identity export (port the connection and key to set the same repo up on another
  device).** Rejected — it exports _setup_, not the repository; secrets would still have to
  come from a re-clone or sync. A different goal.
- **Plaintext / third-party-format export (a decrypted gopass directory or CSV).** Rejected
  for v1 — writing decrypted secrets to disk conflicts with the principle that decrypted
  content never rests on disk outside an operation, and it is a large security surface. Out
  of scope.
- **Always passphrase-wrap the bundle.** Rejected as the default — see the footgun argument
  above; the recoverability of a survival copy should hinge only on the existing identity.
- **Snapshot (HEAD only) instead of full history.** Rejected — it discards revisions and
  audit history, which is a core reason a repository (vs. a flat export) is worth keeping.
  Full history is the faithful "repository" export.

## Effort

Medium. Producing a Git bundle and cloning from one are standard git operations, but the
storage layer today expresses itself through its storage-backend abstraction and a working
tree rather than raw git handles, so bundle creation and bundle-clone need a clean
backend-level capability added without touching any decryption path. The Android / desktop
save-and-pick surface already exists (reused from attachment export). The import flow reuses
the existing clone + identity-setup steps with the bundle as the source. (human: ~3–5 days /
CC: ~2–3 sessions)

## Depends on / Supersedes

- Builds on `005-git-storage` (a repository is a git repo; the clone + identity-setup flow).
- Complements `009-multi-repository` — import becomes a third "add vault" source, and export
  is the safe-egress path that makes removing a local-only vault acceptable. **Independent
  of it** — export can ship before multi-repository (import is deferred — R087).
- Import — the symmetric restore — is **deferred to R087**, gated on a non-git backend
  (`R046`); on git-only it is redundant with clone/sync, so v1 ships export alone.
- Reuses the existing save / pick file surface (attachment export).
