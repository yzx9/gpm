# Filename-encrypting storage backend

**Priority:** P3
**Status:** Draft
**Phase:** Future

## What

Add an optional storage backend that encrypts secret *names* and *structure* on
top of an underlying storage backend. Secret blobs are stored under hashed names
with the directory tree flattened to a single level, and a name-to-location map
is persisted as an encrypted lookup table at the store root; listing is served
from the decrypted map rather than by scanning the filesystem. Revision control
and content IO delegate to the wrapped backend. It composes with both the git
backend and the externally-synced backend (0046).

## Why

A gopass store encrypts secret contents but not secret metadata. The names,
folder structure, and update cadence are plaintext in the working tree and in
remote history. Anyone with read access to the remote — or a forensic dump of the
working tree — learns which services you hold credentials for, how they are
organized, and when they change, even though the contents are opaque.

This exposure is worst exactly where 0046 points gpm: a store synced to a
third-party cloud hands plaintext names, tree, and timestamps to an untrusted
provider. It is also present, to a lesser degree, with a self-hosted git remote.
gopass addresses this with an optional filename-encrypting backend; gpm has no
equivalent, and the cloud-folder backend makes the gap materially worse.

## Context

**The reference model — gopass's filename-encrypting backend.** gopass's backend
is a decorator: it wraps another storage backend (by default git) and translates
only names. A secret's name is hashed to a fixed-length filename; the actual
content blob is stored under that hash with the directory hierarchy flattened
(all blobs at one level; the tree exists only as keys in the map). The
name-to-hash map is itself encrypted with the store's age recipients and written
as a single lookup file at the root, so it is opaque without the identity — the
same trust boundary as the secret contents. Listing returns the map's keys, not a
filesystem scan. Revision-control operations (stage, commit, push, pull) pass
through to the wrapped backend, with names translated to hashes at staging time
and the map file always staged alongside its blobs.

**Why a decorator, not baked in.** Layering name encryption as a decorator over a
storage backend — rather than folding it into every backend — mirrors gopass and
limits blast radius: only this layer knows about name encryption, and every
backend below (git, the 0046 externally-synced backend) is unchanged. It composes
freely: encrypted names over a cloud-folder backend is exactly the configuration
that protects metadata on a third-party sync.

**Compatibility with gopass is a deliberate format match.** A gpm store
interoperates with a desktop gopass store that uses the same filename-encrypting
backend on the same synced location only if gpm reproduces gopass's on-disk
scheme — the hash function, the map file's format and location, and the
encryption of the map. Matching it is the design goal; diverging (a different
hash, a keyed hash) breaks interop.

**Known limitation — dictionary-attackable names.** gopass's scheme uses an
unkeyed hash, so the hashed filenames reveal structure (the tree is flattened)
but not, on their own, the names. However, secret names are often low-entropy and
drawn from a small dictionary (a service or site name); an attacker who can see
the hashed filenames can build a table of common names and reverse them — without
the identity, without breaking the encryption. The map file itself is encrypted,
so this only applies to names recoverable by guessing. The layer therefore hides
the tree structure and uncommon names for free, but does not protect
dictionary-attackable names. A keyed variant (deriving a hash key from the
identity) would close the dictionary hole but breaks gopass compatibility; that
tradeoff is the central open question, not a settled decision.

**Cost shape.** Every write re-encrypts and rewrites the entire map, so per-write
cost grows with the store size; acceptable for personal stores, a concern for
large shared ones. The flattened layout also forfeits the per-directory recipients
partitioning gopass allows (subdirectory `.gpg-id`), since directories no longer
exist on disk.

**Threat-model impact — metadata, not contents.** Secret contents stay encrypted
exactly as today; this layer touches only names and structure. The encrypted map
becomes a new secret-bearing artifact and inherits the same at-rest protection
the identity and configuration already get. The dictionary limitation above is
the one new, honest caveat.

## Alternatives considered

1. **Keyed hash (HMAC over names with a key derived from the identity).** Closes
   the dictionary-attack hole — hashed names become unrecoverable without the
   identity. Rejected as the default because it breaks on-disk compatibility with
   gopass's filename-encrypting backend, which is the main reason to match its
   scheme at all. Kept open as an opt-in "gpm-only, stronger" mode if gopass
   interop turns out not to matter to users.

2. **Encrypt names per-directory, preserving tree structure.** Rejected: it leaks
   the tree shape (how many secrets per folder, folder names recoverable
   structurally) that flattening hides, and it is further from gopass's scheme.

3. **Fold name encryption into the cloud-folder backend (0046) rather than a
   separate decorator.** Rejected: it couples metadata protection to one backend,
   prevents using it over git, and duplicates gopass's separation. The decorator
   composes with every backend.

4. **Do nothing — accept plaintext metadata.** Rejected as the long-term answer
   for the cloud-folder case, where names go to a third party. Acceptable as a
   documented limitation until this lands, mirroring how gopass treats the feature
   as optional.

## Effort

Medium. One new decorator backend over the storage abstraction, map load/save and
encryption, name hashing and translation at the content and staging boundaries,
and a setup-time choice (encrypted vs plaintext names). The bulk of the risk is
format-compat testing against desktop gopass, and the dictionary-vs-compat
decision is a design call, not engineering.

## Depends on / Supersedes

Composes with `0046-pluggable-fs-storage-backend.md` (most useful over the
externally-synced backend, which is where plaintext metadata is most exposed) and
with the git backend. Matches gopass's filename-encrypting backend's on-disk
scheme for interop.
