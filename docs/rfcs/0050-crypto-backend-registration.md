# Crypto backend registration

**Priority:** P3
**Status:** Blocked
**Phase:** Future

## What

A registration mechanism for crypto backends, parallel to the storage one —
deferred until a second crypto backend actually appears and forces the question.
Today there is one crypto backend (age), and it lives inside rustpass, so there
is no consumer for a registration seam. This RFC records the decision to wait:
when a second crypto backend arrives (the GPG/OpenPGP backend), revisit whether
crypto needs the registration mechanism the storage side has, or whether
internal dispatch suffices.

## Why

A registration seam earns its keep only when a backend's implementation lives
outside the crate that uses it, or when there are enough backends that a central
dispatch is worth abstracting. Neither is true for crypto today. The age backend
is a pure-Rust implementation inside rustpass, and the planned GPG backend is
also pure-Rust and will live inside rustpass — chosen precisely because it
cross-compiles to Android without an external binary. There is no crypto backend
whose implementation rustpass cannot host, so there is no external
implementation to register, and no consumer forcing the abstraction.

Building the mechanism now would be designing a registration seam from a single
implementation — the same trap that shaped this codebase's earlier abstractions
the wrong way when attempted from one example. The honest move is to block until
a real consumer (the GPG backend) arrives and tells us whether crypto wants a
registry at all, or whether a typed internal selection is enough.

## Context

**Crypto's backends are internal; storage's are not.** The storage side has a
backend (the cloud-folder backend) whose implementation must live in the app
layer because it bridges to Kotlin; that external implementation is what forces
the storage registry. Crypto has no such backend: every crypto backend is, or is
planned to be, a pure-Rust implementation inside rustpass. Without an external
implementation, rustpass can construct any crypto backend itself, and selection
is an internal decision — a typed backend-kind field resolved after unlock — not
a registry lookup.

**The shared part is late binding, not the mechanism.** Both crypto and storage
keep "which backend" in the sealed repository configuration, unreadable until
unlock, and both resolve it after unlock. What differs is the dispatch: storage
looks up a registry because its backends can be external; crypto matches
internally because its backends cannot. Imposing the storage registry on crypto
would add an indirection with no backend behind it.

**The consumer that unblocks this.** The GPG/OpenPGP backend is the event that
makes this decision real: a second crypto backend with a different identity and
recipient model, needing construction-time selection. When it lands, the
question becomes concrete — does crypto want a registration seam (for symmetry,
or for a hypothetical external crypto backend), or is a typed internal selection
sufficient? Until then the question has no answer that is not speculation.

## Alternatives considered

1. **Impose the storage registration mechanism on crypto now.** Rejected: there
   is no consumer, so it is an abstraction over one implementation, and it
   imports machinery (a registry, an extension namespace) that crypto has no
   external backend to populate. Over-engineering.

2. **Decide the crypto mechanism now anyway, ahead of the second backend.**
   Rejected: a backend-selection shape guessed from a single implementation is
   the documented failure mode of this codebase's earlier abstractions. Shape it
   when the second backend arrives, not before.

3. **Never add a crypto registration mechanism.** Not rejected — it remains a
   live possibility. If the GPG backend lands and internal typed selection
   handles it cleanly, this RFC may be deprecated rather than implemented. The
   decision is genuinely open until the consumer exists.

## Effort

Unknown until the consumer arrives; blocked. When the GPG backend lands, the
work is either small (a typed internal selection, if that suffices) or medium
(a real registration seam, if a reason for one appears). This RFC records the
deferral, not a plan to build.

## Depends on / Supersedes

Blocked on a consumer: the GPG/OpenPGP backend
(`0036-gpg-crypto-backend.md`). Reassess when it lands. Parallel to
`0049-storage-backend-registration.md`, which has a consumer (the cloud-folder
backend) and so is not blocked.
