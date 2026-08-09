# Import a repository from an export

**Priority:** P2
**Status:** Blocked
**Phase:** Future

## What

The symmetric counterpart to repository export (R078): bring a previously
exported repository artifact back in as a new, restorable vault. Export gets a
vault's encrypted data off the device as a portable artifact; import restores
such an artifact to a working vault, completing the loop that makes an export a
real survival copy rather than a one-way egress. Import is a storage operation —
it never decrypts — and is followed by the normal identity-setup step, because
the artifact's secrets stay encrypted to the original recipients and the
importing user must supply an identity that matches one of them.

Serves `docs/specs/005-git-storage` (the repo-connection lifecycle) as a third
"add vault" source alongside clone-from-URL and create-new.

## Why

Import exists to make an export restorable. On its own that argues for shipping
it with export — but its value turns out to depend on which backend the artifact
came from, and on git-only (the sole backend today) that value is weak enough to
defer. This RFC records the analysis and the gate at which import earns its
place.

**On the git backend, import is nearly a no-op over existing machinery.** A
full-history bundle is restored by cloning it — the same clone capability setup
already uses, which already handles local sources with no credentials. An
imported git bundle is therefore not a new capability; it is the existing clone
pointed at a local file, plus the existing identity-setup. The consumers of an
exported git bundle are desktop `git` and `gopass`, which restore it themselves
and do not need gpm to import it.

**The only gpm-side case that needs in-app import is restoring onto a phone** —
a lost phone, a phone-to-phone handoff, an air-gapped restore. That case is real
but narrow: the realistic "second device" path for a local-only vault is to add
a remote (R071) and sync, not to carry a bundle file between phones. So while
git-side import is cheap to build, it has no strong home in a git-only world.

**Import earns its place when a non-git backend exists.** A backend without
git's revision control has no clone and no sync — for such a backend,
export/import is the genuine data-movement and restore primitive, not a thin
wrapper over clone. That is the condition under which import stops being
redundant and becomes load-bearing. The first such backend is the
Storage-Access-Framework backend (R046); until it lands, every backend is git,
and git already covers restore via clone and sync.

## Context

**No unlock, by construction — symmetric with export.** Restoring an artifact is
a storage operation: cloning a bundle (git) or replaying a snapshot (a non-git
backend) copies bytes without decrypting. The importing user supplies a matching
identity afterward, exactly as the clone-from-URL flow already does. So import,
like export, works under App Lock and never places a secret in memory.

**The git mechanism is cheap; the non-git mechanism is not yet designed.** For
git, restoring a bundle is the existing clone pointed at a local source — no new
storage-backend capability, just the file-pick source (already present for the
attachment and diagnostics flows) wired into the clone + identity-setup path.
For a non-git backend, restore is not "a clone"; it is replaying an exported
snapshot into a freshly provisioned backend, and its shape depends on how that
backend's export is defined. That mechanism is deliberately left unspecified
here, to be designed when a non-git backend is concrete.

**Leak surface equals export's.** Import reads only what export wrote — the same
encrypted content and metadata (entry paths, structure, commit messages for a
bundle) the artifact already carries. Import adds no new exposure.

**Independent of multi-repository.** Import is an alternate first-run / "add
vault" source; it does not depend on the multi-repository feature (009) and
reuses the same clone + identity-setup flow. When 009 lands, import becomes the
third "add vault" source.

## Alternatives considered

- **Ship git-bundle import now, alongside export.** Rejected as the v1 shape: on
  git it is redundant — clone already restores a bundle, desktop consumers
  restore it themselves, and the phone-side restore case is narrow and largely
  covered by add-remote (R071). Building it now would add a feature with no
  strong user on the only backend that exists.
- **Make export a snapshot (current state only) so import is "drop files into a
  new vault."** Not the chosen export shape. R078 ships a full-history bundle
  (the faithful "export the repository," gopass-compatible, carrying audit
  history). A snapshot import only arises if a non-git backend's export is
  snapshot-shaped — which is exactly the condition that also gates this RFC.
- **Never build import; rely on desktop git/gopass and add-remote forever.**
  Rejected as a permanent stance. It leaves no in-app restore path for any
  backend that lacks git sync. Acceptable only while git is the sole backend —
  which is precisely the gate this RFC records.

## Effort

Small on git (the existing clone + file-pick source + identity-setup; no new
storage-backend capability). Larger and TBD on a non-git backend, where
restore-from-snapshot must be designed alongside that backend's export. Not
started.

## Depends on / Supersedes

- The symmetric counterpart to **R078** (export a repository). Export ships;
  import is gated.
- Blocked on a non-git backend — the first being the **SAF / pluggable-filesystem
  backend (R046)**. Reassess when such a backend lands; at that point the
  non-git restore mechanism needs design.
- Reuses the clone + identity-setup flow from **005-git-storage**; becomes a
  third "add vault" source alongside **009-multi-repository**.
- The phone-side case it would otherwise serve is largely covered by **R071**
  (add a remote to a local-only store) for the realistic second-device path.
