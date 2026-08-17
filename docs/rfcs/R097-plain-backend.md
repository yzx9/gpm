# Open gopass `plain` (no-encryption) stores

**Priority:** P2
**Status:** Draft
**Phase:** Next

## What

Add a third crypto backend, `plain`, mirroring gopass's first-class
`--crypto plain` backend: encryption and decryption are identity functions,
entries live as bare `.txt` files with byte-exact bodies, and the store's
recipients marker is a content-ignored sentinel. The only entry path is
**cloning an existing gopass plain store** — gpm does not offer to create one,
and bulk conversion between encrypted and plain stores is out of scope. After
cloning (and an explicit consent gate), a plain store gets the full read/write
flow, including sync.

This is an experimental, compatibility-driven backend, not a user-facing
feature; it serves no `docs/specs/` PRD by maintainer decision.

## Why

gopass compatibility is a hard constraint, and gopass itself ships `plain` as a
supported crypto backend (`init` / `clone` / `setup` / `convert` all accept it).
Today gpm cannot open such a store at all — it surfaces as an unknown crypto
kind. A user who keeps a gopass plain store (shared non-secret material,
greppable recovery data, or a store where at-rest encryption is deliberately
forgone) is locked out of gpm entirely.

Secondarily, the clone-time backend detection this feature needs anyway fixes
a latent gap: a freshly cloned GPG store today lists nothing until a GPG
identity is saved, because the backend is defaulted rather than detected.

## Context

**Wire format is fixed by gopass.** Entry `foo/bar` is file `foo/bar.txt` whose
body is the secret, byte for byte, with no framing; the recipients file is the
`.plain-id` marker, whose contents gopass ignores (the recipient set is a
hardcoded sentinel); there is no identity and nothing to unlock. gpm must match
this format exactly or entries will be invisible / corrupted on round-trip.

**Detection is set-based, not a pick.** Clone completion probes the well-known
markers (`.gpg-id`, `.plain-id`, `.age-recipients`) and produces the set of
backends the repository supports. An empty set defaults to age (today's
behavior). A single hit pins that backend. Entry-extension counts are never a
detection signal — `.txt` in particular is far too generic — though they are
shown as evidence when the user must choose.

**Ambiguity asks, it does not error.** When multiple markers are present, the
registration gate generalizes into a backend chooser that shows the evidence
and the orphan consequence (choosing one backend makes entries of the other
extensions invisible to list and search — the same state the existing
conflicting-secrets guard refuses). This mirrors gopass's own loader mechanism,
which recognizes a backend by its marker's presence.

**Consent is captured where gpm introduces the store.** One hard gate before
registering a cloned plain store (cancel cleans up the working copy); after
that, a lightweight inline confirmation on every create/edit, worded to cover
both facts at the moment of risk — the entry is stored unencrypted _and_, if
sync is on, pushed to the store's remote. No ambient badge; reads are untouched.
The ethical stance is informed consent, which is also gopass's own: the owner
chose an unencrypted store; gpm's job is to make that choice visible at the
moments it matters, not to relitigate it.

**A plain store has no identity, and the store layer must not demand one.**
This is the load-bearing change: every secret read/write path today acquires
the identity before reaching the backend, and fails when none is configured.
For plain, identity acquisition short-circuits at that seam — one place, so
every current and future call site (including sync's decrypt-and-re-encrypt
replay) is covered. On top of that, the crypto-backend contract widens
identity into a backend-interpreted, optional input: backends that structurally
require an identity fail closed when it is absent. The design philosophy is
that signature semantics belong to the backend — a wrong-typed or missing
identity is a backend runtime error, not a compile-time capability gate.

**Threat model: unchanged where it matters, honestly narrowed where it
doesn't.** Secrets still never reach the WebView; copy stays native; config and
store metadata sealing is independent of the secret crypto backend, so a plain
store on Android still keeps its sealed repo config. What changes is scoped to
plain stores: their entries are, by the store owner's explicit choice,
unencrypted at rest and pushed as-is to the store's git remote. This is
consistent with the line R078 drew — gpm does not decrypt-and-export; it reads
stores that were never encrypted. The full threat-model rewrite (SECURITY.md /
AGENTS.md) is deferred until plain stops being experimental.

**GPG clone behavior changes shape, not meaning.** Detection pins GPG at clone
instead of defaulting to age until a GPG identity is saved. The existing
identity-type-driven kind correction is retained as a self-healing fallback
rather than removed. End-to-end GPG clone verification is an acceptance item,
not an afterthought.

**Acceptance.** (1) gopass plain interop round-trip — clone, list, read, edit,
sync; (2) GPG clone end-to-end after the detection change; (3) multi-marker
store resolves by asking, with correct orphan warnings; (4) a plain store
completes setup with no identity step.

## Alternatives considered

- **Dev/test-only no-op backend** (behind a compile flag): rejected — the goal
  is real interop with existing gopass plain stores; test ergonomics may fall
  out of the backend for free, but they are not the point.
- **Read-only plain support**: rejected — gopass plain stores are living
  stores; a read-only slice would be reworked immediately upon the first edit
  request.
- **In-setup creation of plain stores**: deferred, not rejected — creation
  consent UX, newcomer-proof entry gating, and the threat-model rewrite come
  with it, and none are needed for the clone-only experimental scope.
- **Bulk convert between encrypted and plain** (gopass `convert`): rejected for
  now — mass decrypt/re-encrypt is a separate, much larger surface, in the same
  family as the decrypted-output flows R078 already declined.
- **Compile-time capability split of the backend contract** (separate
  identity-capable sub-trait): rejected — trait splits should follow observed
  variance in the backend population, and there is exactly one binary dimension
  today (has-identity) while the store holds a single backend serving both
  directions. Revisit only if an encrypt-only or decrypt-only backend ever
  appears; at that point the new dimension forces the split on its own.
- **Erroring on ambiguous multi-marker stores**: superseded during design —
  asking the user beats erroring, and the registration gate the flow already
  needs generalizes into the chooser for free.
- **Ambient "unencrypted" badge**: declined — the per-write confirmation
  carries the information at the moment of risk without permanent UI chrome,
  and reads need no protection (viewing plaintext risks nothing new).

## Effort

The backend addition itself is compiler-guided (the typed kind enum's
exhaustive matches force every site to handle the new variant). The two real
pieces are the identity-skip at the store layer and clone-time detection with
the chooser UX. ~1–2 weeks human; ~2 CC sessions (rustpass core + detection,
then app-layer gate/confirmation UX and acceptance tests).

## Depends on / Supersedes

Depends on the typed-BackendKind repo-config refactor (merged). Same
compatibility family as R093/R095 (pass-store line) but independent of them.
Nothing superseded.
