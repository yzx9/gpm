# Whole-application export format

**Priority:** P2
**Status:** Draft
**Phase:** Future
**Revision:** 1

## What

Define the portable, self-describing format for a **gpm whole-application backup**: a single
archive that can carry, in one versioned envelope, the app's non-secret settings and one or
more repositories (each its own encrypted payload). The format is designed for **compatibility**:
today's single-repository export (R078) is a valid _minimal instance_ of it (no settings, one
repository, no secrets), and future settings-export and multi-repository-export emit richer
instances of the same schema without breaking older readers.

This RFC owns the **format and its compatibility rules only**. It does not specify how
settings-export or multi-repository-export are produced — those are future features that emit
this format. It serves no single feature spec; it is the format layer beneath R078 (repository
export), the future multi-repository export, and the future settings export.

## Why

A repository export alone (R078) is useful, but a user moving devices or recovering from loss
wants the _whole app_ — every vault, plus the preferences that make a fresh install feel like
home — in one restorable artifact. Defining the general format now, before multi-repository
ships, lets the single-repository export already speak it: the format is exercised by the one
export that exists today, and by the time settings and multi-repository export arrive the
schema is settled, so an importer never has to guess what a file is from its name or by sniffing
bytes.

## Context

**Envelope = one self-describing archive.** A whole-app export is a single archive (a gzip
tarball today; a tolerant reader detects the container by magic bytes, not the extension)
containing a manifest and the payload files. The manifest declares what the archive is and how
to read it; the payload is the repositories (and, in future, the settings). Self-describing so a
file found months later — by the user, by desktop `gopass`, or by gpm's own importer — is
identifiable without guessing.

**Manifest schema (v1).** One top-level object: a fixed `type` marker (`gpm.export`), a `version`
(`1`), an optional `settings` block (absent in v1 producers), and a `repositories` array of 1..N
entries. Each repository entry carries its own descriptors — `format` (the inner payload kind:
`git-bundle` today, `saf-snapshot` in future), `backend` (`storage`/`crypto`), its payload
filename, and an optional `secrets` slot (see R089). The current single-repository export emits
`settings` omitted and `repositories: [one entry]`. The archive also carries a human README
(rendered in the supported locales at export time) so it is self-documenting for the
"found-in-two-years" case.

**Compatibility — tolerant reader, additive extensions stay v1.** Readers MUST ignore unknown
manifest fields and default missing optional blocks. So additive extensions — the `settings`
block, `repositories` growing from 1 to N, new per-repository descriptors, the future
`saf-snapshot` payload format — all stay `version: 1`. A `version: 2` is reserved for a
_breaking_ schema change only. This makes today's single-repository export
**forward-compatible** (a v1 reader ignores a future `settings` block) and **backward-compatible**
(a future reader handles a v1 file with no settings).

**One envelope across backends; `format` carries the divergence.** The envelope and manifest are
backend-agnostic; a repository's `format` field declares its payload kind. A git repository's
payload is a git bundle (full encrypted history); a future SAF repository's payload would be a
snapshot (current encrypted files). "Different backends, different payload formats" is contained
to each repository entry, not the envelope — and an importer dispatches on `format`.

**Settings: structure now, fields later.** The `settings` block is an optional, extensible,
non-secret container for app-level preferences. Its exact fields are defined when settings-export
ships; v1 reserves only the structure. Per-repository configuration (remote, commit identity,
authenticity) is _not_ a global setting — it rides with each repository entry, mirroring gpm's
own app-config vs repository-config split.

**The optional per-repository `secrets` slot.** An export MAY include a repository's
identity/credentials in its entry's `secrets` slot. Because secrets are sensitive, the rules for
what may appear there and when encryption is required are owned by the export-encryption RFC
(R089); this RFC only reserves the slot and the invariant that **no raw key ever appears
unencrypted** in an export.

## Alternatives considered

- **Two separate formats — a "repository export" format and a "whole-app export" format.**
  Rejected: it would stop the single-repository export from being the minimal instance of the
  whole-app format, fragment importers, and force a migration. One generalized schema with a
  minimal v1 instance is cleaner.
- **Bake the secrets-safety rules into this RFC.** Rejected: those rules depend on the encryption
  capability (R089). Co-locating them there keeps this RFC a pure format spec and lets the safety
  rules evolve with encryption.
- **Enumerate the `settings` fields now.** Rejected as premature: settings-export is not built,
  so reserving the structure is enough for compatibility, and enumerating fields would couple
  this RFC to unbuilt behavior.

## Effort

Small for the spec itself; the cost lands in the producers. R078 already emits the minimal
instance; settings-export and multi-repository-export are future features. (human: spec only /
CC: spec)

## Depends on / Supersedes

- The format R078's single-repository export conforms to; R078 is the v1 minimal producer.
- The export-encryption RFC (R089) owns the per-repository `secrets` safety rules.
- Composes with multi-repository (R080) — multi-repository is what makes `repositories: [N]`
  meaningful, and R080 already flags whole-vault export as a future capability that builds on it.
- Restore (import) is R087 (deferred); an importer reads this manifest and dispatches on each
  repository's `format`.
