---
pm: Zexin Yuan
created: 2026-07-15
revision: 1
scope: git
---

# 005 — Git Storage & Sync

> Status: Partial · Last verified: 2026-08-10
> Core shipped; several forward-looking reliability items not yet done.

## 1. Introduction

The storage and sync foundation: the vault is a Git repository, auto-sync publishes
changes as they happen, and commits can be signed-and-verified and audited. Includes
the lifecycle of the repo connection (set up / reconfigure).

## 2. Motivation / Objective

Self-hosted, no third-party cloud; sync across devices / people through your own Git
repo; keep history trustworthy (commit authenticity); default to "publish on change,
recoverable on collision."

## 3. Use Cases

- **Jordan** this is core to. They self-host the Git repo (their Forgejo), clone it to
  the phone, and auto-sync publishes every change. Their daily reality is exactly that
  gopass-compat + git-sync loop — write on desktop, read / write on phone, both sides
  always in sync, and either side failing to read what the other wrote counts as a bug.
  When they share the repo (with a partner), they turn commit verification up to
  Enforce, so a compromised remote still can't sneak a forged entry in; and they dig
  through history to audit.
- **Casey** builds a local vault from scratch; the Git layer is completely hidden from
  them — they have no idea a repo is behind it. Local is enough for them; only if they
  one day add a second device does "sync" surface (and even then, guided).

## 4. Key Aspects

### Product Design

- Local write → auto-sync publishes (changed means pushed); a conflict is the explicit
  result of a rejected push (no silent overwrite); authenticity and encryption are
  independent of each other; the repo connection is its own lifecycle.

### Functionality

- Repo setup + reconfiguration; auto-sync + manual sync + conflict handling;
  SSH / HTTPS (SSH-key generation + paste); commit authenticity (SSH + GPG,
  Off / Audit / Enforce) + trusted keys; history (global pagination + per-entry
  revision).

### Compatibility

- gopass repo layout; standard Git protocol; SSH + GPG/OpenPGP signing (the GPG side is
  shared with 004).

### Interactive

- Pull-to-refresh + progress bar + cancellable; conflict dialog (see 002); trusted-key
  paste / import; history pagination + load-more.

### Adaptive

- Plugin keys can run on desktop but not Android (cross-ref 003); background sync runs on
  Android's deferred scheduler (WorkManager, periodic + network-constrained) — desktop has
  no equivalent, so it is Android-only.

### Security

See <./security.md>.

### Reliability

- **Known holes (this feature's forward focus):** a save built on a stale read can
  silently overwrite a newer remote version; a push in progress can't be cancelled.
- Conflicts are recoverable; no data is lost; a missing or undecryptable key degrades
  back to re-setup.

## 5. Open Questions & Key Decisions

- How to detect, before writing, that "this save is based on a stale read" (to avoid
  silently overwriting the remote); making a push in progress cancellable.
- Conflict split is settled: experience in 002, mechanism in 005.

## 6. Roadmap

- **Shipped:** repo setup, auto-sync + conflict, manual sync, SSH/HTTPS, signature
  verification (SSH+GPG), trusted keys, global history pagination, per-entry
  revision history (list + view + copy, path-bound, graceful undecryptable),
  recipients root index, periodic background sync (pull-only, Android).
- **Next / Future:** stale-read protection, cancellable push, repo reconfiguration,
  provenance tracking.
