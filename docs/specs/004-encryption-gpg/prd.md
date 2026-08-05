---
pm: Zexin Yuan
created: 2026-07-15
version: 1.0.0
scope: gpg
---

# 004 — GPG Encryption

> Status: In flight · Last verified: 2026-08-05
> Current: the backend is implemented, interops with system gopass, and is wired through
> the Store for read and write (`Store::get`/`Store::set` route to `GpgBackend` once
> `repo.json` selects it). Remaining: the setup sub-flow (no path writes `crypto:"gpg"`
> or accepts a GPG identity — setup rejects PGP keys), the keyring-management UI, and a
> write-side Store integration test.

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

- The backend is implemented, interops with system gopass, and is wired through the
  Store for both read and write. **Not yet done:** the setup sub-flow (setup rejects
  PGP keys; no path configures a GPG store), the keyring-management UI, and write-side
  Store integration coverage.

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
  write, proven against system-gpg fixtures.
- **Now:** the setup sub-flow (GPG keygen/import + persisting `crypto:"gpg"`), the
  keyring-management UI, and a write-side Store integration test.
