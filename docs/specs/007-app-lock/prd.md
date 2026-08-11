---
pm: Zexin Yuan
created: 2026-07-15
version: 1.0.0
scope: lock
---

# 007 — App Lock & Auto-lock

> Status: Shipped · Last verified: 2026-07-30 · Related: A003
> Authoritative relock signal not yet done.

## 1. Introduction

App-level locking: App Lock (biometric) + Auto-lock (Immediate / Idle / Never) +
encryption at rest. On lock, the vault key (which gates the identity) is wiped
immediately, and no sensitive data is left behind.

## 2. Motivation / Objective

If the phone is lost or lent, stop someone from reading the vault outright; and
encryption at rest means whoever pulls the phone's files only sees ciphertext.

## 3. Use Cases

- **Jordan** turns App Lock on and sets Auto-lock to Immediate — they are particular
  about the identity key living in memory only for the single operation in flight. As a
  technical user, they tune the lock timing themselves.
- **Casey** this is where their "even if I lose the phone, no one flips through my
  passwords" peace of mind comes from — one fingerprint press opens it, and it stays
  locked the rest of the time. They turn it on and configure nothing else; encryption at
  rest is invisible to them but protecting them the whole time.

## 4. Key Aspects

### Product Design

- App Lock uses biometrics to gate **the whole app** (not just a single password);
  Auto-lock has three modes; under Immediate the decrypted identity lives in memory only
  for the duration of a single operation.

### Functionality

- App Lock, Auto-lock's three modes + activity-reset timer, encryption at rest, the
  App Lock idle re-lock (in-app idle, and — for `After(N)` — a resume grace so a
  quick app switch within N no longer re-locks; R058), and identity coupling to the
  gate (all shipped); an authoritative relock signal (not done).

### Compatibility

- (Thin) gopass has no equivalent — this is purely a gpm / Android platform layer.

### Interactive

- Lock-screen overlay; unlock dialog; biometric first; an Auto-lock setting; the App
  Lock idle re-lock mask (non-dismissable, tap to unlock — no auto-prompt); a single
  "Lock & Identity" settings page.

### Adaptive

- App Lock + encryption at rest are Android only; desktop has no equivalent (local is
  plaintext there — this asymmetry is recorded); iOS deferred.

### Security

See <./security.md>.

### Reliability

- A missing or undecryptable key degrades to re-setup (no silent failure); while a
  biometric prompt is up, relock is not triggered (to avoid a race).

## 5. Open Questions & Key Decisions

- The source of an authoritative relock signal (hardening on top of today's best
  effort); the scope of the Idle timer's activity reset.

## 6. Roadmap

- **Shipped:** App Lock, Auto-lock's three modes, encryption at rest, activity-reset
  timer, the App Lock in-app idle re-lock, identity coupling to the gate (when
  Auto-Unlock is on).
- **Future:** authoritative relock signal (hardening the dependence on OEM / WebView).
