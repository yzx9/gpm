# App-Launch Biometric Lock (Resume Gate)

**Priority:** P2
**Status:** Draft
**Phase:** Next

## What

An opt-in biometric gate that challenges the user every time gpm returns to the foreground — cold start and resume from background alike — independent of the existing 5-minute identity auto-lock and of the at-rest master key. It behaves like a banking-app lock screen: one biometric prompt to get back in, then the normal session resumes underneath.

## Why

Today the only lock is the 5-minute inactivity timer on the cached identity. Android keeps the process alive across brief backgrounding, so switching away and back within the window re-enters the app with no challenge — there is no lock screen between the user and their entries. Users expect a password manager to demand biometrics on every app open, and the 5-minute timer never re-triggers on a quick app switch, so the protection it provides is weaker than it feels.

## Context

gpm already has two distinct Android Keystore mechanisms. This gate is a deliberate **third, UI-layer** mechanism that touches neither:

- The **at-rest master key** is sealed with an auth-free, hardware-backed key and unsealed silently at startup. It must stay auth-free: it defends the local config and identity files against a _read_ attacker, and binding it to biometrics would brick the whole store (git credentials, trust set, identity) on a fingerprint change, rather than just invalidating a passphrase seal that self-heals.
- The **identity passphrase** can optionally be sealed behind a biometric-gated key, surfaced today as "biometric unlock" on the identity lock overlay.

The new gate is orthogonal: a foreground trigger that asks for biometric before showing app content, regardless of whether the identity is currently cached.

Two design constraints drive the shape:

1. **At most one biometric prompt per foreground.** When the gate fires and the identity happens to be locked at the same moment, the single biometric authentication also unseals the identity passphrase (reusing the existing biometric-gated Keystore) — one prompt does both jobs ("dual-purpose"). When the identity is still unlocked, or biometric-for-identity is off, or the identity is plaintext, the gate is a plain, non-crypto-bound biometric check. The identity lock overlay is suppressed while the gate is active, so the two can never race to show competing prompts.
2. **No lockout.** The toggle can only be enabled when a strong biometric is available; it auto-disables if biometric is later removed; an encrypted identity falls back to its passphrase; a plaintext identity's only escape is a full reset (documented, since plaintext has no softer fallback).

The gate ignores resume signals while a biometric prompt is already in flight, so the prompt's own show/dismiss cannot re-trigger the gate (loop guard).

**Threat model:** consistent with the existing one (local opportunistic access). The gate adds a UX-level challenge against someone who picks up an unlocked device and reopens the app within the 5-minute window. It does not change the at-rest or identity crypto guarantees, and it does not defend a process running as the app (an explicit non-goal). The toggle itself is sealed at rest by the existing auth-free master key, so it is readable at startup without prompting.

## Alternatives considered

- **Merge the gate into the at-rest master key (one biometric-gated key for everything).** Rejected: a biometric-gated master key is invalidated by fingerprint changes, which would brick the entire store and force re-setup (re-clone, re-enter the git token). The current split isolates that fragility to the passphrase seal, which self-heals with one passphrase re-entry. The two layers defend different threats and must stay separate.
- **Cold-start-only gate.** Rejected: Android process persistence means a cold start is rare; most returns are warm resumes a cold-start-only gate would miss. It would feel broken as a lock screen.
- **Two independent biometric prompts (gate + identity, both gated).** Rejected: strictly worse UX with no security gain; the dual-purpose path already covers the identity unlock when needed.
- **Per-operation biometric (prompt on every copy/show).** Rejected: far too intrusive for a password manager whose primary operation is quick clipboard copies, and unnecessary once a session is established.

## Effort

~M (human) / ~M (CC): one new plugin capability (plain biometric check + resume lifecycle hook), a small backend command module, a persisted config flag, and a frontend lock-screen overlay + state. The resume → frontend event path is the one piece of new wiring to validate early.

## Depends on / Supersedes

Builds on `0002-keystore-biometric.md` (the biometric-gated identity Keystore reused for the dual-purpose path). Does not supersede it.
