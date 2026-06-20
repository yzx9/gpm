# Create a new store from scratch

**Priority:** P2
**Status:** Draft
**Phase:** Next

## What

Add a first-run path that creates a brand-new, gopass-compatible age store **on device** as an
alternative to cloning an existing one. The flow generates an identity (age x25519 or SSH, the
user's choice), seeds the store's initial recipients file with that single recipient, runs a
`git init` with a gopass-style initial commit, and — optionally — configures a git remote and
pushes. Everything after creation (encrypt / commit / sync) reuses the machinery gpm already has;
only identity generation, recipients-file authoring, and store initialization are new.

## Why

gpm is clone-only. A user with no existing gopass store cannot start without external tooling —
install the gopass CLI, generate a key, `gopass init`, push to a git host — a heavy tax,
especially on mobile. The asymmetry is glaring: gpm can already encrypt, commit, and push new
secrets into a store, but it cannot create the store it writes to. This closes that gap and
unlocks a true first-run "I want to start using a password manager today" path, with no host and
no second tool required.

## Context

**Current behavior.** Setup is clone-first; after a clone the store is assumed fully formed and
nothing is initialized. The recipients file is read-only today — gpm consumes the list the cloned
store ships and never writes or amends it. Identities are imported (pasted or picked), not
generated in-app; the one exception is SSH keys, which gpm already generates. There is no
production age-identity generator.

**gopass prior art (verified against gopass's source, not paraphrased).** The recipients file
`.age-recipients` is the sole init marker — a store counts as initialized iff that file exists at
its root. `Init` requires at least one recipient that has a matching private identity, writes the
file, and commits with the message **"Initialized Store for <recipients>"**. gopass's own `setup`
wizard is the direct template: generate an identity → initialize locally (write recipients +
`git init`) → an optional **"Do you want to add a git remote?"** prompt that **defaults to "no"**.
A single recipient paired with a single identity is a fully valid, gopass-readable store. The
identity lives per-user, never inside the store; the recipients file holds only public keys, so it
is safe to commit and push.

**Design notes.**

- The chosen shape deliberately mirrors gopass's `setup` wizard, so a store gpm creates is
  indistinguishable from one gopass creates — users can mix tools or migrate freely.
- Write `.age-recipients` (gopass-canonical, and the format gpm already reads) rather than
  `.gopass-recipients`; seed it with the user's own recipient, matching gopass's single-recipient
  default.
- Local-first: a remote is not required to create, matching gopass. A user can begin immediately
  and add a remote / sync later, instead of being forced to produce a git host on first run.
- The generated identity stays on device through the existing identity storage (at-rest
  encryption; biometric-keystore gating on Android). Nothing private is ever committed to the
  store.
- Asymmetry to record: gopass additionally appends the local keyring's own recipients on every
  encrypt, beyond what `.age-recipients` lists. For a single-identity gpm store this is a
  non-issue, but it is the kind of cross-implementation detail to fix in writing before any future
  multi-recipient work (see `0005-multi-identity`).
- Upstream caveat: gopass still labels the age backend "experimental … on-disk format likely to
  change." The `.age-recipients` format itself has been stable for years, but gpm inherits that
  caveat.

## Alternatives considered

- **Document external gopass bootstrap instead of building in-app.** Rejected: the mobile
  onboarding tax is precisely what this removes, and the deliverable is an in-app path.
- **Require a git remote at creation.** Rejected: reintroduces the friction being removed and
  diverges from gopass's local-first model; chose remote-optional.
- **Generate only an age x25519 identity (no SSH).** Rejected as the sole option: offer SSH too,
  since gpm already supports both identity types and already generates SSH keys — there is no
  reason to withhold it on the create path.
- **Write `.gopass-recipients` instead of `.age-recipients`.** Rejected: `.age-recipients` is
  gopass-canonical and the format gpm already reads; this maximizes interoperability with gopass
  and the bare `age` CLI.
- **Skip `git init` (plain directory).** Rejected: gpm's history/sync features and gopass
  compatibility expect a git repo, and `git init` is cheap and expected. gopass itself defaults to
  a git-backed store.

## Effort

~1-2 days (human) / ~45-60 min (CC). New pieces: an age-native identity generator, a
recipients-file writer, store initialization (`git init` + initial commit), an optional
remote-and-push branch, a "create" branch in the setup UI alongside "clone", and tests
(rustpass integration + frontend).

## Depends on / Supersedes

Keeps the single-identity stance of `0005-multi-identity` (creation seeds one recipient;
multi-recipient / team stores stay deferred there). Composes with `0016-recipients-pinning` (a
freshly created store's recipients can be TOFU-pinned on first write, same as a cloned one).
Touches the setup flow of `0004-reconfiguration-flow` and the identity system behind SSH keygen.
Does not supersede anything.
