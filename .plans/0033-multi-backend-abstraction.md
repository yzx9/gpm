# Multi-backend abstraction: swappable Crypto + Storage/RCS backends

**Priority:** P1
**Status:** Accepted
**Phase:** Next

## What

rustpass is hardcoded to two libraries — age for encryption and git2/libgit2 for sync — with the
`Store` facade calling both directly and no seam between them. This RFC extracts two swappable backend
interfaces mirroring gopass's `Crypto` + `Storage` split, with the existing age and git implementations
as the sole backends behind those interfaces. It is a refactor: no new backend, no behavior change. The
seams are shaped so a second backend could arrive later without redesign.

## Why

Today there is no abstraction boundary between the store and its two dependencies. The revision-control
layer is a concrete module of free functions with nothing behind it, the store performs ad-hoc working-
tree file I/O inline, and it reaches directly into the git library for commit-signature authenticity
checks. Consequences:

- The sync layer can't be unit-tested in isolation — every test has to stand up a real git repo.
- `Store` knows about two libraries it has no reason to care about, so every backend concern (recipient
  parsing, conflict classification, transport quirks) leaks into the facade.
- rustpass diverges from gopass's architecture, which we deliberately mirror for format compatibility
  and to make porting future gopass features cheap.

Extracting the interfaces organizes rustpass around gopass's backend boundaries, makes the sync layer
testable at the seam, and prepares the ground for a second crypto or RCS backend + a gopass-style
registry without doing that work now.

## Context

**The reference model.** gopass splits its internals into two interface axes, not three: `Crypto`
(encrypt/decrypt, recipients, identity management) and `Storage` (file ops). `Storage` *embeds* an
unexported RCS interface — there is no standalone revision-control trait; every storage backend must
satisfy the RCS methods. The pure-filesystem backend stubs them; the git/fossil/jujutsu backends
implement both. gopass's ADR-3 drafted splitting `Storage` and RCS into two separate traits and
**deferred** it, on the reasoning that nearly every storage backend is also an RCS backend, so the
merged interface is correct in practice. rustpass follows the merged model: one `StorageBackend`
interface carrying both file-op and RCS methods, plus a `CryptoBackend` interface. A separate RCS trait
is explicitly not chosen — it would force a future pure-storage backend to stub RCS methods it cannot
deliver, for no present benefit.

**At-rest encryption is orthogonal and stays put.** The AEAD layer that protects the app's own local
config files (the repo configuration and the identity) is a local-config-protection concern, tied to the
Android Keystore master key. It does not protect the repository's age-encrypted secrets and does not
vary with the crypto or storage backend. It stays in the configuration layer; moving it into a backend
would conflate the repository working tree with application state.

**Built on decouple-sync.** This work layers on top of the in-flight decouple-sync effort (local-only
writes, an autosync orchestrator holding a single write lock across pull → write → push, three-way sync
classification, and keep-mine conflict resolution). That work is concrete git with no traits yet, so the
starting point is a decoupled-but-concrete sync layer. The autosync orchestrator and its critical
section stay at the store level; the backend methods are critical-section-agnostic.

**Threat-model impact — the keep-mine contract.** When a sync diverges and the user chooses "keep
mine," the local secret must be decrypted and re-encrypted for the *current* recipient set onto the
remote tip — never blindly rebased. A blind rebase replays old ciphertext and leaves the secret
encrypted to stale recipients, a silent access-control regression (a rotated key or new teammate cannot
decrypt a secret they should own). To make that regression structurally impossible rather than a
comment someone might ignore, the storage interface exposes keep-mine as two steps — a *plan* that
returns entry names and metadata only, and a *finalize* that writes re-encrypted ciphertext — with the
store doing the decrypt-and-re-encrypt between them. Plaintext never enters the storage layer, because
the interface gives it no method that accepts plaintext.

**Conscious tradeoff.** Two independent reviews flagged that extracting trait-object interfaces at N=1
(one crypto backend, one storage backend, no runtime selection this round) is premature: the
trait-object dispatch is tax with no payoff, and trait shapes guessed from a single implementation tend
to be reworked when a second one arrives. The tradeoff is accepted — the value taken in exchange is
exact gopass architectural parity and cheaper future feature-ports. The trait shapes are expected to be
revised when a real second backend informs them; that rework is budgeted, not avoided.

## Alternatives considered

1. **Three traits (split Storage and RCS).** gopass's deferred ADR-3 plan: export RCS as its own
   interface, have the store hold a separate RCS handle, give the pure-filesystem backend a no-op RCS.
   Rejected: the merged model matches gopass today, and rustpass has no legacy revision-control call
   sites that would make the split worth its cost.
2. **Concrete component structs now, trait objects later.** Carve concrete age and git backend
   components with explicit method ownership and clean seams, but defer the `dyn`-dispatch traits until
   a second backend informs the shape — capturing the coupling-cleanup and testability value without
   the premature-abstraction cost. Preferred by both reviewers; rejected by the user in favour of
   literal interface abstraction now (gopass parity).
3. **Defer the entire refactor until a second backend is concrete.** Rejected: the store↔git coupling
   cleanup and the testability win are valuable regardless of how many backends exist, and decouple-sync
   landing is a natural inflection point to introduce the seams.
4. **Authenticity as a third, optional backend capability.** Model commit-signature verification as a
   capability interface that some backends implement and others don't (a pure-storage backend cannot
   verify commits). Deferred to its own RFC, not rejected: authenticity is a full-stack feature spanning
   the backend, the Tauri command layer, two frontend pages, the threat model, and its own prior RFC, so
   it gets a dedicated review rather than riding this refactor. This refactor keeps authenticity working
   unchanged behind the existing signing module, with its direct library calls relocated there.

## Effort

~5 human-days / ~3h CC, across this RFC plus three implementation PRs (crypto axis, storage file-ops
axis, RCS fold-in).

## Depends on / Supersedes

Depends on `0028-decoupled-writes-autosync` and `0032-cancellable-saves` (the decouple-sync branch)
merging first — this refactor rebases onto that base. Relates to `0009-gpg-signature-verification`
(authenticity), whose optional-capability redesign is explicitly deferred here.
