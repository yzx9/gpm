---
pm: Zexin Yuan
created: 2026-07-15
version: 1.0.0
scope: id
---

# 006 — Identities & Trust

> Status: Partial · Last verified: 2026-07-28
> Single identity shipped; multiple identities and recipients pinning not yet done.

## 1. Introduction

Identities and recipients: which key decrypts your vault, and who gets encrypted to.
Today there is a single identity; planned are multiple identities plus tamper
protection on the recipients file.

## 2. Motivation / Objective

Repos shared across people / devices need multi-identity routing; if a malicious key is
injected into the recipients file, new entries get silently encrypted to an attacker —
so the recipients need to be pinned; and "undecryptable" should be a graceful state,
not a crash.

## 3. Use Cases

- **Jordan (sharing the vault, power use)** runs one repo with several roles — work /
  personal / partner directories each routed to a different decrypting identity. When
  they share the repo, they pin the recipients so that even if someone slips an
  attacker's key into the shared recipients file, new entries won't be silently
  encrypted to them. These are the power defenses they set up for "sharing."
- **Casey** is fine with a single identity; these defenses are on by default and
  completely invisible to them — they don't have to configure anything.

## 4. Key Aspects

### Product Design

- An identity is a labeled set (add / remove / name); recipients pinning is a
  **file-level** defense, independent of 005's commit signing — it still applies when
  authenticity is off; "undecryptable" is not an error.

### Functionality

- Single identity (shipped); multiple identities + routing; recipients first-trust +
  drift confirmation; overwrite safety — refuse to overwrite a remote entry the current
  identity can't decrypt.

### Compatibility

- Follows gopass's recipients-file convention; the pin / confirm semantics align with
  gopass.

### Interactive

- Identity-management UI, recipients-drift review / confirm UI (both to be built).

### Adaptive

- Multi-identity routing is pure software, consistent across platforms.

### Security

See <./security.md>.

### Reliability

- An undecryptable entry shows its metadata, and its ciphertext is not delivered to the
  UI.

## 5. Open Questions & Key Decisions

- Routing fallback (when nothing matches: try all vs. error outright); where recipients
  pinning is stored; when to split the identity cache out on its own (gated on hardware
  keys).

## 6. Roadmap

- **Shipped:** single identity, recipients root-index resolution.
- **Next:** recipients pinning + confirmation, multiple identities.
