---
pm: Zexin Yuan
created: 2026-07-15
version: 1.0.0
---

# 004 — GPG Encryption

> Status: In flight · Last verified: 2026-07-28
> Current: the backend is implemented and interops with system gopass, but it is not
> wired into the main flow yet — no setup, no keyring-management UI.

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

- The backend is implemented and interops with system gopass; **not yet done:** wiring
  it into the main flow, the setup sub-flow, the keyring-management UI.

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

- **Shipped:** the GPG side of signature verification (shared with 005).
- **Now:** wire into the main flow + setup + keyring UI.
