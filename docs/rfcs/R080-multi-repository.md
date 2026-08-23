# Multi-repository architecture

**Priority:** P1
**Status:** Draft
**Phase:** Next
**Revision:** 1

## What

Make gpm hold several repositories ("vaults") on one device, where the user is in one vault
at a time and switches between them. Each repository keeps its own remote, identity,
authenticity policy, and sync settings; one App Lock gates the whole app. This RFC fixes the
design that the `009-multi-repository` spec requires: how the repositories are structured in
process and on disk, how operations address a repository, how locking and background sync
generalize, and how an existing single-repository user upgrades.

## Why

Every layer of gpm today assumes exactly one repository — one store facade held by the app
state, one sealed repository config, one identity, one cross-process lock, one command set
that implicitly targets "the" repository. A001 deferred multi-repository to post-MVP, and
A003 deliberately isolated the per-repository configuration unit so that this work would be a
relocation into a per-repository directory, not a re-architecture. This RFC is that
relocation: it records the load-bearing design decisions and the two places the product chose
the more explicit, more uniform option over the lower-churn one.

## Context

**One store facade per repository, unchanged.** The store facade is already a clean,
singly-slotted, per-repository unit — its crypto backend, storage backend, decrypted-identity
cache, write lock, and autosync flag each belong to one repository. Rather than generalize
one facade to hold many repositories (turning every slot into a map), the design holds a
**registry of independent store facades, one per repository**, each self-contained and rooted
at its own configuration directory. The facade's internals do not change; the existing "one
builder can produce many stores" affordance is exactly this. The storage backend is already
parameterized per operation, so the only leak was upstream, at the facade — and a registry of
facades sidesteps it entirely.

**Each repository is a self-contained directory; the set lives in the application config.**
Every repository's durable state — its sealed repository config (remote, credentials,
authenticity trust-set, backend choice), its sealed identity, its clone, and its cross-process
lock — lives under its own sub-directory keyed by a stable, opaque identifier generated when
the repository is added. The identifier never changes, so renaming a repository (a display
concern) only touches its display name, not its identity, directory, or any reference. The
**set of repositories, their order, and the last-active one are recorded in the application
config** (device-scoped, sealed under the auth-free key, readable at launch) — which is where
A003's placement rule puts device-scoped sealed data by default, avoiding a new sealed slot.
The display name lives with each repository's own config and is derived from its remote URL
until the user renames it.

**All facades are constructed at startup.** Constructing a store facade does no disk I/O —
backends are resolved lazily per operation, the identity cache starts empty, and the
cross-process lock is taken per write. So startup constructs every repository's facade
eagerly from the application config's repository list, and the cost is negligible. Lazy
construction would add on-demand machinery for no real saving.

**Every operation names its repository explicitly.** Each repository-touching operation
carries the repository identifier; the frontend owns which repository is "active" and stamps
the identifier onto every call, so the backend holds no active-repository state. This is
more churn than having the backend track an implicit active repository, but it is uniform
(background sync and management operations address a repository the same way UI operations
do) and keeps the backend stateless about selection. Non-UI callers do not use "active": the
periodic background sync iterates every repository with autosync on.

**Locking and the identity cache stay per-repository; the wipe is global.** Each facade
already owns its decrypted-identity cache, so per-repository caches come for free and need
no extraction. There is one lock policy and one idle timer; when the app locks, the at-rest
vault key is wiped from every facade, which drops every cache. A repository may opt its
identity into "unlock together with the app" via its existing per-repository toggle, so a
trusted repository needs no separate prompt. The deferred wipe that lets a keep-mine
divergence resolve reuse an unlocked identity is naturally per-repository, but an app-lock
wipe overrides it — security ahead of resolve convenience.

**One pair of at-rest keys seals every repository.** A single auth-free master key and a
single vault key (the existing device-level pair) seal every repository's config and
identity; the facades are all constructed with the same keys. Compromising the vault key
compromises every repository's at-rest identity, consistent with the per-device threat model
— defeating the key already means the device's App Lock is broken. This avoids
per-repository keys (many keystore entries and invalidation behaviors) for isolation the
threat model does not reward. The application config (which now also carries the repository
registry) is owned by the existing application-config component, not by any one repository's
facade.

**Background sync fans out.** The periodic, pull-only background sync becomes one
global-cadence job that iterates every repository with autosync on, pull-syncing each.
Per-repository locks come free from per-repository directories; pull-only needs no identity,
so it runs under App Lock as it does today. The divergence/authenticity "attention" markers
become per-repository. This builds on the pure-scheduler work (R077) rather than running
beside it.

**Upgrade is a forward migration with no decryption.** A one-time forward migration (the
permanent registry's next step) runs at startup, before any facade is constructed: it
generates an identifier, moves the single repository's durable state into its new
sub-directory, and records the repository in the application config. Because the keys are
shared, nothing is re-sealed — the identity file moves untouched, and only the application
config is re-sealed under the same key to add the registry. The migration therefore has no
vault-key/App-Lock dependency. The display name is derived from the URL on demand, so the
migration writes nothing extra.

## Alternatives considered

- **Generalize one store facade to hold many repositories.** Rejected — it turns every
  singly-slotted field into a map keyed by repository, a deep rewrite of the facade, and
  fights the existing "many stores from one builder" affordance. A registry of unchanged
  facades gets the same result with no internal rewrite.
- **A separate registry file for the repository set.** Rejected — the registry is
  device-scoped sealed data, which A003's placement rule defaults into the application
  config. A separate file is a new sealed slot the rule would fold away anyway, for marginal
  write-path isolation.
- **Human-readable slug as the repository identifier.** Rejected — it churns on rename
  (directory move, reference rewrite) and collides; an opaque stable identifier keeps renames
  cheap and references stable. The user never sees the directory.
- **Lazy facade construction.** Rejected — facade construction is I/O-free, so lazy loading
  adds on-demand machinery with no meaningful saving.
- **Implicit active repository tracked by the backend.** Rejected (product decision) — an
  explicit per-operation identifier is uniform across UI, background, and management callers,
  and keeps the backend stateless about selection, at the cost of carrying the identifier on
  every call.
- **Flat routing (active repository as session state).** Rejected (product decision) — a
  repository-segmented route makes the active repository the URL's source of truth (no
  separate state to keep in sync), gives each repository a stable URL for future
  deep-linking/autofill, and matches the "explicit everywhere" stance, at the cost of a route
  segment on every screen.
- **Per-repository at-rest keys.** Rejected — the threat model is per-device, so the
  blast-radius isolation is not worth many keystore entries and invalidation behaviors;
  shared keys also make the app-lock wipe simplest.

## Effort

Large. The store facade and storage backend need no internal change — the work is a new
registry layer above them, threading a repository identifier through the operation surface,
the frontend active-repository state and segmented routing, the repository management flow
(add / remove / rename / switch), the background-sync fan-out, and the forward migration.
The genuinely hard parts are the operation-surface threading and the frontend rework; the
per-repository locks, caches, and keys are essentially free from the existing design.
(human: ~2–3 weeks / CC: ~6–10 sessions)

## Depends on / Supersedes

- Serves `009-multi-repository` (the requirements this implements).
- Builds on `A003` (the per-repository config unit this relocates) and `A001` (which deferred
  multi-repository to post-MVP).
- Coordinates with `R077` (pure scheduler — the background-sync fan-out builds on it).
- Coordinates with the App-Lock / at-rest-key model (`R064`); the shared-key decision keeps
  that model intact.
- Related to `R005` (multi-identity) — orthogonal axes: multi-repository is vault-level
  isolation, multi-identity is role-level within one repository.
- Related to `R098` (vault connection editing) — credential rotation and same-repo URL edits
  are connection-level operations on one repository, layered on the per-vault Settings surface
  this design introduces.
- The repository export/import RFC (`R078`) becomes a third "add vault" source under this
  design.
- Exporting **every vault in one backup artifact** is a future capability this design unlocks
  (multi-repository is what makes "all vaults" a meaningful set to back up at once); the
  single-repository case ships first via `R078`, and the multi-repository export envelope is
  scoped in a separate export-format RFC.
