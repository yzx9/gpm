# Export a repository via Git bundle

**Priority:** P2
**Status:** Draft
**Phase:** Next

## What

Let a user export an entire repository as a portable, self-describing **archive** whose
payload is a full-history **Git bundle** — the bundle _is_ the repository, its encrypted
history made portable. The archive is the minimal instance of the whole-application export
format (`R088`): a manifest (declaring what the file is, with a version) plus the bundle plus
a human README, in one file that can be handed to another device, kept as a backup, or opened
in desktop gopass. (The symmetric counterpart — re-importing as a new repository — is analyzed
and **deferred to R087**; it earns its place only once a non-git backend exists.) Bundling is
a Git operation that **never decrypts**, so export does not require the vault to be unlocked
and the bundle's secrets stay encrypted. The bundle exposes the same metadata (entry paths,
structure, commit messages) that the git remote already exposes; the manifest and README add
only non-secret description — no new leak surface.

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
access to the git remote can already see. The `R088` envelope (manifest + README) adds only
non-secret description, so the default export adds **no exposure beyond what the remote model
already assumes**. Recoverability of a default (unencrypted) export depends only on the age
identity the user already manages — the envelope carries no new secret that can be lost.
Hiding the metadata is the job of the _optional_ export-encryption layer (`R089`), which wraps
the whole archive to a recipient; it is opt-in, not the default, so a survival copy's
recoverability is never gated on an extra passphrase or recipient key the user might lose.

**Where the export goes.** On Android the user picks a destination through the system
save-file picker; on desktop through the standard save dialog — the same surface already used
for attachment export, so no new platform plumbing. The saved file is the `R088` archive (e.g.
`gpm-export.zip`). To restore on desktop, a user extracts the archive and `git clone`s the
bundle inside (the README explains how); gpm's own restore (`R087`, deferred) reads the
archive and its manifest directly. Packaging several files behind one save mirrors the
diagnostics-export archive.

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
- **Always encrypt the export.** Rejected as the default — see the footgun argument above;
  recoverability of a survival copy should hinge only on the existing age identity. Encryption
  to a recipient (`R089`) is opt-in — for transmission through untrusted channels or
  partner-transfer — not the default shape.
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
- Conforms to the whole-application export format (`R088`); this single-repository export is
  R088's minimal v1 producer (no settings, one repository, no secrets).
- The optional export-encryption layer (`R089`) wraps the archive to a recipient; it is opt-in
  and governs the (future) per-repository `secrets` slot.
