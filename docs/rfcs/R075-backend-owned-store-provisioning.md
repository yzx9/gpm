# Backend-owned store provisioning

**Priority:** P2
**Status:** Draft
**Phase:** Next

## What

A store's first-time setup — clone an existing repository, create a new one, or
configure one together with an identity — is today a set of operations on the
Store facade that hard-codes the git backend and drives git-specific
revision-control machinery (clone, init, remote-add, the initial commit)
directly. This RFC moves that setup — call it _provisioning_ — off the facade and
onto the storage backend: each backend owns how it comes into existence, and the
Store consumes an already-provisioned backend rather than being the thing that
knows "this is a git clone." Provisioning is kept separate from recipients
seeding: a provisioned backend is live but empty, and the recipients index (and,
for revision-control backends, the initial commit that records it) is configured
as a distinct step afterward. Serves the git-storage feature
(`docs/specs/005-git-storage`) and the pluggable-storage-backend direction
(RFCs 046–051).

## Why

The setup surface is the last place the Store facade assumes git. Everywhere
else the facade is being made backend-agnostic — content operations ride a
backend the Store never names, resolve reconstructs a backend from a persisted
type it treats as opaque, and provenance is on track to become the backend's
answer rather than the facade's git assumption. Setup is the exception: it is
where the facade decides the backend _is_ git and reaches into revision control
to create the repository. That is the coupling the pluggable-backend work exists
to dissolve, and it is the coupling a second backend cannot live with — there is
no clone, no init, and no initial commit on a Storage-Access-Framework document
tree, so a facade that drives setup as "a git clone plus a seeded-recipients
commit" has no way to provision any other backend.

The surface bakes in two git assumptions at once. The first is mechanism: setup
assumes revision control. The second is sequencing: it assumes a new store is
born by initializing revision control, seeding the recipients index, and
committing it in one fused act. Both are git's shape. A SAF store is born by
acquiring a document-tree handle — no revision control, no commit — and a
local-only store by creating a directory. Provisioning those is strictly simpler
than provisioning git, but only if provisioning is the backend's own
responsibility with its own shape, not a git-shaped recipe the facade runs.

Fusing provisioning with recipients seeding also blurs a clean boundary.
Provisioning is "make a backend usable"; seeding recipients is "configure the
crypto index." They have different owners (storage versus crypto/store),
different failure modes, and — for a non-revision backend — different shapes
(seeding is a file write with no following commit). Pulling them apart lets each
be reasoned about and tested on its own, and makes the SAF path honest: its
provisioning is handle acquisition and nothing more.

## Context

**Two construction moments, not one.** A backend and its root come into being in
two distinct moments that the stateful-backend model must both cover. The later
moment is _resolve_: on each unlock after the first, the Store rebuilds a backend
from a persisted type and root token (RFC 051's subject). The first moment is
_provision_: the one-time act that creates the root in the first place — clone,
init, or handle acquisition — and establishes the type and root that resolve will
later replay. RFC 051 owns resolve; this RFC owns provision. They share the same
stateful backend (a backend owning its root); this RFC is how that backend and
its root are born, 051 is how they are rebuilt.

**Provisioning is not a uniform trait method; it rides the existing dispatch.**
The inputs differ per backend — clone takes a URL and credentials, SAF
acquisition takes a document-tree URI, local-only creation takes nothing — so
provisioning cannot be one method on a shared interface with a single signature.
It belongs where each backend is already dispatched on its own terms: the git
built-in owns clone and init natively, and an extension backend's construction —
which today turns a root token into a backend — is already where an extension
acquires whatever its root requires. The move is therefore an orchestration
relocation, not a new shared method: the knowledge "this setup is a git clone"
leaves the facade and lands where git is already a named built-in, and the
facade stops naming any backend type during setup.

**git does not ride the filesystem abstraction, and neither does its
provisioning.** RFC 046 keeps git on a real filesystem with its own revision
control, and gives non-git backends a thin filesystem trait instead. Provisioning
follows the same split: git's provisioning (clone, init, remote-add, the
recipients commit) is git's own, on the built-in side; the filesystem trait
carries no provisioning notion, because its backends are provisioned by acquiring
a handle or a directory, not by revision control. A SAF or local-only backend
thus has no clone, no commit, and no "initial commit" step to fake — its
provisioning is genuinely the simpler thing the design wants.

**Recipients seeding is a separate, post-provision step.** A provisioned backend
is usable but empty: a cloned store already carries the remote's recipients
index (seeding is a no-op); a freshly initialized revision-control store has an
empty working tree and no commits; an acquired SAF handle is an empty tree.
Seeding the recipients index — writing the crypto backend's recipients file — is
then a distinct configuration step owned by the store/crypto layer against
whatever backend was provisioned. For a revision-control backend that step may
include the initial commit that records the index (gopass's "Initialized Store"
commit); for a non-revision backend it is a plain file write with no commit.
Either way provisioning has already finished and touched no recipients, which is
the separation this RFC requires.

**The three setup flows decompose along the storage/crypto seam.** Cloning an
existing remote is pure provisioning. Creating a new store is provisioning (init
or acquire) followed by recipients seeding (and, for revision backends, the
initial commit and an optional remote). Configuring a store together with an
identity is two things sharing one operation today: an identity-validation and
persistence half that is a crypto/store concern and stays on the facade, and a
clone half that is provisioning. Moving the three flows therefore moves their
_storage_ responsibilities onto the backend; the identity half of configure is
not storage and does not move. The facade ends up consuming a provisioned backend
for all three, and the backend reports the persistable type and root that resolve
will later need — while the facade, which owns the sealed repository config,
persists them.

**The orphan-recipient invariant is preserved.** Today a new store is not pushed
until both its recipients index and its identity are durable, so a remote can
never receive a store whose recipient's identity has been lost locally.
Decoupling provisioning from recipients seeding does not weaken this: seeding
still completes before any push, and the deferred first push stays deferred until
the identity is durable. The same holds for the partial-setup cleanup that today
removes a half-born repository on failure: it follows provisioning, so it moves
with it. The change is in where the steps live, not in their ordering or the
invariants they uphold.

**Threat-model impact — none.** No secret crosses a new boundary. Provisioning
moves clone, init, and handle acquisition that already handle only public
material (URLs, credentials the facade already trusts the backend with, the
public recipients line), and recipients seeding is a relocation of an existing
write. A provisioned backend is trusted for the whole repository for the whole
session, exactly as a resolved one is today; widening a backend's reachable
state from "one operation's worth" to "the backend's lifetime" changes no trust
boundary — the same conclusion the stateful-backend model reaches.

## Alternatives considered

1. **Leave provisioning on the facade and branch it per backend.** The facade
   would detect the backend kind and run a different setup recipe per kind.
   Rejected: it re-creates the very coupling the pluggable-backend work exists to
   remove — the facade would still enumerate backends and know git's recipe — and
   every new backend would require a facade change. It is the setup-side form of
   the branching-on-backend-kind pattern the provenance RFC (048) rejects for the
   same reason.

2. **Add a single `provision` method to the shared storage trait.** One uniform
   method every backend implements. Rejected: provisioning's inputs are
   backend-specific (a URL and credentials for clone, a document-tree URI for
   SAF, nothing for local-only), so a uniform signature is either a
   lowest-common-denominator bag of optional parameters or a trait method most
   backends cannot honestly implement. It also pushes revision-control notions
   onto the filesystem trait that RFC 046 deliberately keeps clean. Provisioning
   rides the existing per-backend dispatch, not a new shared method.

3. **Keep provisioning and recipients seeding fused, but move the fused act to
   the backend.** Each backend would own "initialize and seed my own recipients"
   as one step. Rejected: it gives the storage backend responsibility for the
   crypto recipients index, crossing the storage/crypto seam the rest of the
   design keeps clean, and it forces every backend's provisioning to know about
   recipients — including SAF, whose provisioning should be nothing but handle
   acquisition. The separation this RFC requires is precisely the point.

4. **Defer until a second backend exists, then shape provisioning from two
   implementations.** Rejected as timing: the setup-on-facade coupling is the
   blocker that makes a second backend hard to add in the first place, and the
   resolve/provision split is a foundational decision the stateful-backend family
   should make once, up front — the same reasoning RFC 051 gives for making the
   state-model decision ahead of the backends that stand on it.

## Effort

Medium. ~2–3 days human / ~moderate CC. Relocating the three setup flows'
storage responsibilities off the facade onto the backend dispatch, splitting
recipients seeding into a distinct post-provision step (including detaching the
initial commit from provisioning on the revision-control create path), and
reworking the setup command layer to consume a provisioned backend. No new
backend ships here — the git built-in is the only provisioning implementation —
mirroring how RFC 051 moves the trait without shipping a new backend. The
non-git provisioning value is unlocked once RFC 046's backends land.

## Depends on / Supersedes

Depends on `0051-storage-backend-state-model.md` — provisioning and resolve are
the two moments of the same stateful backend, and a backend that does not own
its root has nowhere to own its provisioning. Depends on the storage-backend
registration mechanism (in code: `StorageRegistry`) for the built-in/extension
dispatch that provisioning rides. Relates to `0046-pluggable-fs-storage-backend.md`, whose
non-git backends are where decoupled, simpler provisioning earns its keep, and
whose "git does not ride the abstraction" decision this RFC mirrors on the setup
side. The setup-side companion to `0048-backend-owned-provenance-verification`:
both move a git-assumed responsibility off the facade and behind the backend.
