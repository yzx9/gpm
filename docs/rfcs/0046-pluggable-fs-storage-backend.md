# Pluggable filesystem storage backend (SAF & local-only)

**Priority:** P2
**Status:** Draft
**Phase:** Future

## What

Introduce a thin filesystem abstraction between the store's content logic and
the bytes on disk, so the storage layer can ride a non-POSIX backend. Two
concrete backends land behind it: an Android Storage-Access-Framework-backed
backend that lives in a store pointed at a cloud-folder document tree
(Dropbox / OneDrive / Drive sync the folder; gpm just reads and writes), and a
local-only no-RCS backend — gopass's `fs` equivalent. The git backend keeps its
own real-filesystem working tree unchanged. The store constructor gains an
injection seam so the app layer supplies the backend rather than the store
hard-wiring git.

## Why

Today the storage layer couples three things that gopass keeps separate:
working-tree file IO, the real POSIX filesystem those IO calls assume, and git's
revision control. Every content operation is keyed on a concrete filesystem
path, and the only backend is the git one. That makes a whole class of storage
impossible to express — anything that isn't "a real directory that libgit2 can
open."

The motivating case is Android. Scoped storage removed the shared POSIX folder
that desktop gopass relies on for its `fs` + external-sync-tool pattern
(Dropbox / OneDrive / Box watching a folder). The only mainstream way to reach a
cloud-synced location on Android is the Storage Access Framework — a URI/tree
model with no real paths, exposed as an Android API the pure-Rust backend crate
cannot call directly. So a SAF-backed store needs its implementation in the
app/plugin layer while the store orchestration stays in the backend crate. That
split is impossible without a filesystem abstraction in the backend crate that
the app layer can implement.

Separately, gpm has no gopass-`fs` equivalent: a store that is local-only and
lets an external tool own sync. Users who already run Syncthing or a cloud
client have no way to point gpm at a folder the way desktop gopass users can.

## Context

**The reference model — gopass separates storage from RCS.** gopass's `fs`
backend is intentionally RCS-free: every revision-control method is a no-op, and
the backend exists "for users who manage versioning through an external mechanism
(a FUSE overlay, a network filesystem with versioning)." The git / fossil /
jujutsu backends each bundle a real filesystem plus an SCM. The
filename-encrypting `cryptfs` backend is a decorator over any of them. gpm's
current single backend is the bundled kind; this RFC adds the RCS-free kind and
the seam to host it.

**Dependency inversion is the only clean seam.** SAF (`ContentResolver`,
`DocumentsContract`) is a Java/Kotlin API; the backend crate is pure Rust with no
JVM. So the filesystem trait must be defined in the backend crate and
*implemented* in the app layer (a Rust shim in a Tauri plugin bridging to Kotlin,
the same shape as the existing file-picker plugin). The alternative — a callback
backend where the store calls up into the app layer per file operation —
duplicates the store's orchestration in the app layer and is far uglier. The
backend crate already has a small read-only precedent for this shape (the
repo-file view the crypto backend reads recipients through), so the direction is
not new to the codebase.

**The trait stays thin; atomicity and path containment are not part of it.** A
filesystem abstraction that bakes in POSIX-only notions — atomic rename, or
canonicalize-style containment checks — cannot be implemented on SAF, which
offers neither. So the trait carries only primitives (read, write, delete,
existence, make-directory, list by relative path), and each implementation owns
its root token and its own write/containment strategy. The standard
implementation keeps its current temp-file-plus-rename atomicity; the SAF
implementation does best-effort single-stream writes and accepts a documented
torn-write window. The root is an opaque token (a path for the standard impl, a
persisted document-tree URI for the SAF impl), not an absolute filesystem path
threaded through every call.

**git does not ride the abstraction.** libgit2 operates on real directories
through OS syscalls and has no hook to redirect file access to a virtual
filesystem. So the git backend keeps a real-filesystem working tree and is
unaffected by the abstraction; the filesystem trait earns its keep only for the
non-git backends (SAF and local-only). git-over-SAF is explicitly out of scope.

**The security model shifts by backend, not weakens.** On a real filesystem the
backend defends against path traversal and planted symlinks with lexical and
canonicalize containment plus a no-follow probe of the recipients index before
reading it. None of those notions exist on SAF: there are no symlinks, and the
provider kernel-enforces scope (a granted document tree cannot be escaped). So
the SAF backend's containment is the provider's, and the recipients-index
tampering guard — meaningful only where symlinks exist — is delegated to the
implementation, where it stays a real defense on the standard backend and reduces
to provider scoping on SAF. The guarantee (a malicious clone can't shrink the
recipient set by planting a symlink at the index) is preserved on both, by
different means.

**Two real-filesystem assumptions leak above the backend today and must be
cleaned up as part of this work.** First, content operations are keyed on a
concrete filesystem path rather than a backend-owned root token, so the root
must stop being a path the store threads around and become the backend's own
state. Second, the recipients-index liveness guard is a direct no-follow metadata
probe done outside any backend, because it must not follow symlinks; that
assumption has to move behind the filesystem trait so the SAF backend can supply
its own (provider-scoped) equivalent. These are the parts of the work that touch
the existing git path, not the new backends.

**gopass on-disk compatibility is preserved by construction.** The file layout —
the secret-name-to-path convention, the crypto backend's extension, the
recipients index at the root — is decided above the filesystem trait, in the
store and crypto layers. A SAF backend that stores bytes at the relative paths
the store asks for produces the same tree as gopass's `fs`: the same `<name>.age`
leaves, the same nested directories (SAF supports nested document trees), the
same `.age-recipients`. A gpm SAF store and a desktop gopass `fs` store pointed
at the same synced folder interoperate directly. The only behavioral divergence
is the lost write atomicity on SAF — a partial write can be visible to a syncing
peer, where desktop `fs` rename is atomic.

**Threat-model note — sync guarantees change.** A SAF-backed store has no
revision history and no conflict detection visible to gpm: the cloud tool
reconciles concurrent edits, typically last-write-wins with silent "conflicted
copy" files. This is a strictly weaker sync guarantee than the git backend's
divergence / keep-mine resolution, and it is the same guarantee desktop gopass
`fs` users already accept. It should be surfaced to the user as a property of
this backend, the way the git backend's stale-read limitation is today. Filename
metadata also rides the cloud in plaintext; protecting it is a separate, layered
backend (see 0047).

## Alternatives considered

1. **Callback backend (store calls up into the app layer per file op).** Rejected:
   duplicates the store's encrypt / decrypt / recipient orchestration on the app
   side and makes every content op an IPC round-trip shaped by the store's
   internals. The trait-injection design keeps orchestration in one place and the
   app layer a pure primitive supplier.

2. **A second storage backend with no separate filesystem trait.** Write the SAF
   backend directly against the existing storage-backend interface, with file ops
   bridging to Kotlin and RCS ops as no-ops. Partially viable and lower-churn, but
   it bakes the concrete-path keying and the POSIX path guards into an interface
   the SAF backend cannot honor, and it leaves no place for a shared local-only
   backend. Rejected in favor of factoring the thin filesystem trait out, which
   gives a real local-only backend for free and keeps the SAF backend honest about
   what it can and cannot do.

3. **Bundle a git binary (Termux-style) for cloud sync.** Rejected: Android 10+
   forbids executing bundled binaries — the same wall that blocks
   age-plugin-yubikey — and it would re-introduce the external-binary dependency
   the libgit2 backend exists to avoid. The cloud-folder path exists precisely for
   users who do not want a git remote at all.

4. **Cloud object storage (S3 / GCS) as a first-class sync backend.** Rejected: it
   is a non-gopass format with no interop, and it contradicts gpm's standing design
   constraint that sync is the user's own files moved by a tool the user chose.
   SAF-to-a-cloud-folder keeps that model ("your files, your sync tool") while
   reaching the cloud.

## Effort

Large. A new trait with two implementations (standard, already mostly present as
today's working-tree logic; SAF, new, bridging to a new Kotlin plugin), a
local-only RCS-free backend, the store constructor injection seam, and — the part
that touches existing code — reworking the concrete-path keying and the
out-of-band recipients-index guard to be backend-neutral. The SAF path also
carries an Android-specific build/verification cost like the other plugins.

## Depends on / Supersedes

The storage-side analog of the crypto multi-backend abstraction (which the GPG
backend exercises). Relates to `0036-gpg-crypto-backend.md` — the crypto backend
that reshaped its trait when a second backend arrived; this is the same lesson on
the storage side (the filesystem trait is shaped by its second, non-POSIX
backend). Composed with `0047-filename-encrypting-storage-backend.md`, which
layers on a storage backend to protect the metadata this backend would otherwise
sync in plaintext.
