---
pm: Zexin Yuan
created: 2026-07-15
version: 1.0.0
scope: age
---

# 003 — age Encryption

> Status: Partial · Last verified: 2026-07-28 · Related: A001
> Core shipped; hardware-key decryption and post-quantum (PQ) decryption not yet done.

## 1. Introduction

The default encryption: age. An age identity or an SSH private key serves as the
decryption identity, the passphrase can be held behind biometrics, and a hardware key
like a YubiKey is another decryption identity (in flight). No system GPG needed; works
on Android.

## 2. Motivation / Objective

No system-GPG dependency, Android-usable; age is gopass's modern default; and a
hardware key keeps the master key from being extracted.

## 3. Use Cases

- **Jordan** is on age by default — the identity is their age identity, or an SSH
  private key they already keep in their dotfiles; after entering the passphrase once,
  they switch to fingerprint unlock. They very much want to decrypt with a YubiKey, so
  the key never leaves the hardware (in flight). When they hit a post-quantum (PQ) or
  plugin-type key, what they want is for gpm to recognize it and tell them plainly
  "not supported yet," not to masquerade it as a parse error.
- **Casey** never sees age — at vault setup they set a passphrase, and from then on
  it's fingerprint unlock. They won't engage with identities, let alone hardware keys.

## 4. Key Aspects

### Product Design

- The identity is the key that decrypts your vault; the passphrase protects the
  identity, and App Lock protects the whole app. "Recognized" and "decryptable" are two
  different things — an unsupported key is not a parse failure.

### Functionality

- age / SSH-private-key decryption + passphrase protection (shipped); encrypting to
  plugin keys (shipped, desktop); recognizing PQ / plugin keys (shipped); decrypting
  with a hardware / plugin identity (not done).

### Compatibility

- age format, compatible with gopass; recognizes common plugin keys like YubiKey.

### Interactive

- Unlock flow; fingerprint first, passphrase folded away; pasted identities are masked.

### Adaptive

- Plugin keys need a subprocess that runs on desktop but can't run on Android — so
  there it surfaces a clear error; full PQ decryption is gated upstream.

### Security

See <./security.md>.

### Reliability

- A missing binary or unsupported key type produces a clear error rather than failing
  silently; a passphrase-protected SSH private key can be retried on failure.

## 5. Open Questions & Key Decisions

- How to settle the abstraction for hardware-key identities.
- PQ route: wait for native upstream support, or implement it ourselves.

## 6. Roadmap

- **Shipped:** age / SSH decryption, passphrase + biometrics, encrypting to plugin
  keys, recognizing PQ / plugin keys.
- **Next:** plugin-identity decryption, native Android YubiKey. **Blocked:** full PQ
  decryption.
