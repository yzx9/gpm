# Crypto backend selection (typed built-in dispatch)

**Priority:** P3
**Status:** Implemented
**Phase:** Current

## What

Crypto backend selection for a store: a typed match on a persisted
`RepoConfig.crypto` field — `None`/`"age"` → `AgeBackend`, `"gpg"` →
`GpgBackend` — resolved lazily post-unlock, mirroring storage's resolve
lifecycle but **without a registry**. The field lives in sealed `repo.json`,
unreadable until app unlock; `Store::resolve_crypto` runs at startup and is
folded into the storage one-shot at app-unlock. An unknown kind surfaces as
`BackendNotAvailable`.

## Why

The second crypto backend (GPG/OpenPGP, RFC 0036) landed, forcing the decision
this RFC had deferred. Both backends are rustpass-internal pure-Rust unit
structs that rustpass constructs itself, so selection is an internal typed
match — not a registry lookup. There is no external crypto backend whose
implementation rustpass cannot host, so there is nothing to register. Typed
internal selection handles GPG cleanly, vindicating the original "wait for the
second backend, then see if typed selection suffices" thesis.

## Context

**Mirror of storage's resolve lifecycle, not its mechanism.** Crypto keeps the
valuable part of the storage pattern — the persisted backend-kind field, the
lazy post-unlock resolve, the `crypto()` accessor that surfaces a specific
error when unresolved — but drops the registry. Storage needs a registry
(`ext:` namespace + factory map) because its cloud-folder backend's
implementation lives outside rustpass (it bridges to Kotlin). Crypto has no
such backend, so imposing storage's registry would be an indirection with no
backend behind it.

**The `crypto()` accessor is fallible.** It returns `BackendNotAvailable` when
the slot is `None` (pre-unlock, after a failed resolve, or after `reset`),
refusing operations the store cannot correctly perform rather than silently
serving a wrong default. Its reachable error paths are narrow — all crypto ops
are gated behind `load_repo_config`, which fails pre-unlock, and an unknown
crypto kind needs manual config corruption — but the explicit refusal is the
same contract `storage()` offers.

**The `ext:` extension seam is deferred.** No `register_crypto` / `ext:` crypto
namespace is built, because there is no external crypto backend to populate
it. The deferral is a bet: it stays cheap only while the next crypto backend is
internal pure-Rust (a third typed match arm). If a future backend is
JNI-bridged, a subprocess, or stateful, typed dispatch breaks and the registry
gets built then. "Deferred" means "not re-written iff the next backend is
internal pure-Rust," not "never."

## Alternatives considered

1. **Impose the storage registry on crypto (register_crypto + `ext:` namespace).**
   Rejected: no external crypto backend exists, so the `ext:` path would be
   dead-in-production code with zero consumers. Add the seam only when a
   consumer (e.g. JNI/PGP) appears — it is additive and non-breaking.

2. **Infallible default-and-swap accessor (always hold `AgeBackend`, swap at
   unlock).** Rejected: it would silently serve the wrong backend on a corrupt
   config (`crypto = "quux"`) instead of a clear error. The fallible accessor's
   cost (a few internal `?` additions, no public-API change) is worth the
   explicit error states.

3. **A separate CAS one-shot for crypto resolve.** Rejected: crypto and storage
   read the same sealed `repo.json` at the same unlock instant, so a separate
   guard buys nothing. Crypto resolve is folded into storage's existing
   one-shot; the shared flag means "both resolved."

## Effort

Done. The typed match, lazy resolve, `crypto()` accessor, reset teardown, and
the startup/post-unlock wiring landed across config, store, and the Tauri app
layer, with an end-to-end integration test proving a `crypto = "gpg"` store
decrypts through `Store::get`.

## Depends on / Supersedes

Resolves the deferral this RFC originally recorded: the second backend
(RFC 0036) arrived and typed internal selection sufficed, so no registry was
built. Parallel to `0049-storage-backend-registration.md`, which keeps its
registry because the cloud-folder storage backend is external.
