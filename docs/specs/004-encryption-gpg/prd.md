---
pm: Zexin Yuan
created: 2026-07-15
revision: 1
scope: gpg
---

# 004 — GPG Encryption

> Status: v1 shipped · Last verified: 2026-08-08
> Current: the backend interops with system gopass and is wired through the Store for
> read and write, AND both setup sub-flows ship — open an existing gopass-GPG store
> (clone → import a GPG secret key → verify its S2K passphrase → use), or create a
> brand-new one by importing a single key (gopass `init`: seeds `.gpg-id` +
> `.public-keys/<token>`, the two init commits, `.gitattributes` + `diff.gpg` config).
> `save_identity` is the crypto-persistence authority that writes `crypto:"gpg"`.
> Remaining: in-app GPG key generation, the recipient-keyring-management UI, and GPG
> paste import (file-pick only).

## 1. Introduction

A second encryption backend: GPG / OpenPGP, compatible with gopass-GPG repositories.
No system gpg needed; works on Android.

## 2. Motivation / Objective

Serve users with GPG-encrypted repos (older repos, or work environments that require
GPG); work without a system gpg, identically on Android and desktop.

## 3. Use Cases

- **Jordan (gopass-GPG variant)** has a GPG-encrypted gopass repo — maybe a legacy one,
  maybe one their job requires to use GPG. They want to open it on the phone: enter the
  GPG passphrase once (then go through biometrics), manage the in-app keyring, and
  trust / import signers' public keys. Casey is untouched by this — they would never
  have a GPG repo.

## 4. Key Aspects

### Product Design

- A self-contained GPG backend compatible with gopass-GPG repos; the identity is a
  private key in the keyring; it sits alongside the age backend and is chosen per-repo
  (not two backends on one repo).

### Functionality

- The backend interops with system gopass and is wired through the Store for both read
  and write; the setup sub-flow ships (open an existing GPG store: clone → import a GPG
  secret key → verify its S2K passphrase → use). **Not yet done:** in-app GPG key
  generation, the recipient-keyring-management UI, and GPG paste import (file-pick only).

### Compatibility

- gopass-GPG repo format; interoperates with system gopass.

### Interactive

- (To be defined) the GPG setup sub-flow and keyring-management UI.

### Adaptive

- No system gpg needed; behaves the same on Android and desktop; the keyring is in-app
  (it never touches the system `~/.gnupg`).

### Security

See <./security.md>.

### Reliability

- An unrecognizable format gives a clear error; a backend problem does not affect the
  main process.

## 5. Open Questions & Key Decisions

- How age / GPG backends are chosen (is per-repo enough); whether that choice is saved
  with the repo; the shape of the keyring UI.

## 6. Roadmap

- **Shipped:** the GPG side of signature verification (shared with 005); the `GpgBackend`
  trait impl (encrypt/decrypt/unlock/recipients/profile) and its Store wiring for read and
  write, proven against system-gpg fixtures; the v1 setup sub-flow (open an existing GPG
  store via file-pick import + S2K verify, with `save_identity` as the crypto-persistence
  authority) and a write-side Store integration test; and the create sub-flow (`Store::
create_gpg_store` — import one key, seed `.gpg-id` + `.public-keys/<token>`, gopass's
  two `init` commits + `.gitattributes`/`diff.gpg` config), proven against the system
  `gpg` CLI.
- **Now:** in-app GPG key generation, the recipient-keyring-management UI, and GPG paste
  import (file-pick only in v1).
