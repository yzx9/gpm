# gopass Binary Interop Tests

**Priority:** P1
**Status:** Accepted
**Phase:** Now

## What

Add end-to-end compatibility tests that drive a real `gopass` binary (age backend) and assert gpm's full stack can read the store it produces — closing the gap left by today's source-mirroring and bare-`age`-only interop. A minimal forward test (gopass writes, gpm decrypts) ships first; the reverse direction and git-sync round-trip are the follow-on matrix.

## Why

gpm's gopass alignment is asserted two ways today, and neither touches a real gopass: (1) reading gopass's source and mirroring its concepts in code, and (2) round-tripping gpm's _own_ output through the standalone `age` CLI. A store actually produced by `gopass` is never exercised, so silent drift in the layers gopass layers _on top of_ age — the recipients file, the secret body convention, git commit/author habits — is invisible until a user hits it.

Driving a real gopass to probe this surfaced concrete divergences on the first try (different initial and save commit messages; an extra gitattributes file gopass writes; author identity taken from git config rather than a fixed app identity) and confirmed the raw-age layer is wire-compatible. None of those is a functional break today, but a compatibility claim that a real binary never validates is a claim that rots in silence.

## Context

**The age layer is already provably shared.** gopass's age backend and the bare `age` CLI are both the reference age implementation, and gpm uses the age crate; the ciphertext is a fixed spec, so gopass-produced blobs decrypt with gpm's age code. The unproven surface is gopass's own conventions, and that is exactly what these tests exercise through gpm's full read path (recipients parse → git clone → age decrypt → secret-body parse).

**gopass is fully isolatable and non-interactively drivable.** Every gopass write can be redirected into a throwaway directory via its config/home environment variables, so a test never touches a developer's real gopass. The one wrinkle is that gopass passphrase-protects its age identity at rest and prompts for that passphrase through `pinentry` on every read; the dev shell ships no pinentry and a test has no TTY. A mock pinentry returning a fixed passphrase over the standard pinentry protocol resolves this cleanly — no TTY, no external dependency.

**Identity injection is the constrained step.** gopass can generate an age identity non-interatively only when given an explicit password; its identity-import command is unreliable, and its recipient-add command rejects an arbitrary pasted `age1…` string (it resolves the argument as a key id). The robust shared-identity technique for the forward direction is to write gpm's recipient directly into the recipients file — gopass's own on-disk format — which gopass honors on every insert (it warns about a missing owner key but encrypts anyway, and does not rewrite the file). gpm then holds the matching private key and decrypts.

**The reverse direction is gated on giving gopass a decryption key.** gopass will not ingest a plaintext age identity, so the two tools cannot share a raw age key both ways. The clean shared identity for bidirectional interop is an SSH-ed25519 key, which both gpm and gopass load natively as an age identity (gopass through its age SSH-key path). The forward test ships without this; reverse and sync adopt it.

**Coverage this cannot provide.** Password generation is non-deterministic, so it is verified by property parity, never byte-equality against gopass output. age-plugin/yubikey paths require the plugin binary on both sides and stay desktop-only. MIME/attachment secrets are a separate, larger gap (see its own RFC).

## Alternatives considered

- **Commit gopass-produced binary fixtures** and assert gpm reads them. Rejected: static, frozen to one gopass version, and they rot silently when gopass changes its conventions — the exact failure mode a live binary test exists to catch.
- **Expand the bare-`age` interop only.** Rejected: already done, and it deliberately bypasses every gopass-specific convention these tests exist to cover.
- **Ship the full forward + reverse + sync matrix in one change.** Deferred: reverse depends on the SSH-key shared-identity setup and sync on a shared bare remote with signature verification; both are real work. A minimal, robust forward test delivers most of the value now and de-risks the rest.

## Effort

~S (human) / ~S (CC) for the forward test and flake wiring. ~M added for the reverse and sync directions.

## Depends on / Supersedes

None. Builds on the existing bare-`age` interop precedent as prior art.
