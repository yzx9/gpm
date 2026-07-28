# Storage backend registration (built-in + ext: extensions)

**Priority:** P2
**Status:** Draft
**Phase:** Next

## What

A registration seam for storage backends that splits backends into two
namespaces. rustpass ships a small set of built-in backends (the git backend and
the local-only backend) natively; backends whose implementation lives outside
rustpass register themselves under a reserved extension namespace. A persisted
backend-type field selects which backend a store uses: built-in names dispatch
natively, extension names dispatch by lookup, and anything else is rejected. The
mechanism is how the app layer supplies rustpass a backend it cannot construct
itself, without rustpass learning what that backend is.

## Why

The cloud-folder backend's implementation cannot live in rustpass: it must call
Android's Storage Access Framework over Kotlin, and rustpass is pure Rust with no
JVM. So that backend has to be supplied to rustpass from the app layer rather
than constructed by it. But rustpass must remain unaware of the cloud-folder
backend specifically — it is one example of a backend rustpass cannot foresee,
and baking its name into rustpass would re-couple the layers the pluggable
backend work exists to separate.

The built-in backends need none of this ceremony: their implementations are in
rustpass, so rustpass constructs them directly. Separating the two with a
reserved namespace earns three things. First, the persisted backend-type field
is self-documenting — a built-in name means "rustpass knows this," an extension
name means "something rustpass does not know." Second, extension names cannot
collide with names rustpass may add as built-ins later. Third, backend
availability becomes a per-build property: a build that does not include the
cloud-folder backend simply does not register it, and a store configured for it
gets a clear "not available" error rather than a silent fallback.

## Context

**Dependency inversion, not a callback.** The seam is a registry the app
populates: each extension backend registers how it is built, and rustpass looks
up the persisted type at resolve time. This keeps the store's orchestration in
rustpass and the app a supplier of construction logic, rather than duplicating
the store's flow in the app per file operation. It mirrors the shape gopass uses
for its backend registry, adapted to Rust (explicit startup registration rather
than global auto-registration).

**The reserved namespace is a contract, not a convention.** Extension names must
carry a fixed prefix; registration rejects names without it, and dispatch treats
any non-prefixed, non-built-in name as an error. The point is to keep the
built-in namespace owned by rustpass and the extension namespace owned by
integrators, so neither side can accidentally shadow the other and so a future
built-in never breaks an existing extension.

**The backend's identity is sealed and resolved late.** Which backend a store
uses, and the token that locates its root, both live in the sealed repository
configuration, unreadable until the app is unlocked. So the registry is
populated at startup (the app knows which backends this build offers), but the
backend is only constructed after unlock, when rustpass can read the persisted
type and root and look them up in the registry. rustpass hands the root token to
the backend opaquely — for built-ins it is a path, for the cloud-folder backend
a document-tree URI; rustpass interprets it only for its own built-ins.

**Per-build availability is a feature, not a gap.** Desktop builds have no
cloud-folder backend (there is no Storage Access Framework outside Android), so
they do not register one. A store configured for the cloud-folder backend,
opened on desktop, fails with a clear "backend not available" error. This is
correct: there is no meaningful desktop fallback for that backend, and the
registry makes the absence explicit rather than papering over it.

**This specifies the injection seam the pluggable-backend RFC names.** That RFC
calls for "an injection seam so the app layer supplies the backend"; this RFC is
the concrete registration mechanism behind that seam — built-in versus
extension, the reserved namespace, the persisted type field. It does not change
what backends exist or the filesystem trait; it is only how a backend is
selected and constructed.

**Why crypto is not done this way.** The crypto backends are all pure-Rust
implementations inside rustpass; there is no external crypto backend driving a
registry. So crypto resolves its backend internally and does not need this
mechanism now. That parallel — and why it is deferred there — is its own RFC.

## Alternatives considered

1. **A single factory the app implements with its own dispatch.** Rejected: it
   centralizes the backend list in one app-side match that must be edited for
   every backend, offers no namespace protection between built-in and extension,
   and gives the app no help with the built-ins rustpass already provides.

2. **A typed enumeration of all backends, including the cloud-folder one, in
   rustpass.** Rejected: it makes rustpass aware of the cloud-folder backend by
   name, re-coupling the layers, and it cannot host a backend whose
   implementation rustpass cannot compile. The reserved-prefix scheme keeps
   rustpass blind to extensions by construction.

3. **Global auto-registration of backends (gopass-style init).** Rejected for
   Rust: it relies on language-level registration hooks Rust does not have
   cleanly. Explicit startup registration is clearer and makes per-build
   availability obvious.

## Effort

Small to medium. One registry with built-in dispatch and an extension map, the
persisted backend-type field with backward-compatible defaults, and the
post-unlock resolve path. The backends themselves and the filesystem trait are
out of scope (they belong to the pluggable-backend RFC).

## Depends on / Supersedes

Specifies the injection seam in `0046-pluggable-fs-storage-backend.md`. The
parallel crypto-side mechanism, and why it is deferred, is
`0050-crypto-backend-registration.md`.
