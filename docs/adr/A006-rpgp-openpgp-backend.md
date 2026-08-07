# A006: rpgp (the `pgp` crate) as the OpenPGP implementation

**Date:** 2026-08-07 · **Status:** Accepted

## Context

gpm needs an OpenPGP implementation for two cross-cutting paths: the GPG/OpenPGP crypto backend for secrets (spec 004) and GPG/OpenPGP commit-signature verification. The choice is foundational and hard to reverse — both paths depend on it for the life of the project, and swapping the library means rewriting both.

The Android target dominates the constraints. Android has no system `gpg` binary, no `gpg-agent`, no `~/.gnupg`, and — since Android 10 — forbids apps from executing bundled binaries. So the implementation must be a library linked into the app, not a binary spawned over IPC, and it must cross-compile to the Android NDK through gpm's existing pure-Rust toolchain. The permissive-license stack gpm already uses (MIT/Apache age) must not pick up a heavier obligation.

## Decision

Use **rpgp** (the pure-Rust `pgp` crate, MIT/Apache-2.0) as gpm's single OpenPGP implementation, shared by the GPG secrets backend and commit-signature verification.

## Consequences

- **Pure Rust, trivial Android cross-compile** — no C crypto to vendor; the existing flake toolchain builds it for the NDK (build-proven during the verification spike), and it runs unchanged on desktop.
- **Permissive license** — no LGPL static-link/relink obligation on a Play-distributed APK; the same licensing posture as the age stack.
- **One implementation, two consumers** — the GPG secrets backend and signature verification share it, so there is one OpenPGP trust boundary and one thing to audit and update.
- **Mobile precedent** — Delta Chat's Rust core uses rpgp across Android/iOS/desktop, the closest production mobile use of the crate; there is no same-stack (Tauri + Android + rpgp) precedent, so confidence rests on rpgp being pure Rust and on Delta Chat's mobile use, both of which have held up empirically.
- **Scoped-out surface (accepted)** — rpgp carries no OpenPGP-card/YubiKey hardware-key path, no Brainpool/LibrePGP/Elgamal algorithms, and no web-of-trust policy layer (key flags/expiry/revocation). None of the missing algorithms arise on a gopass-compatible path; the policy layer is a thin wrapper gpm writes around rpgp. Hardware-token keys are recognized-but-unsupported with an honest error rather than a silent failure (see R030/R043 for the age-YubiKey follow-on; a GPG OpenPGP-card stub is rejected at import). Reconsider only if a concrete need forces a different library.

## Alternatives considered

- **Sequoia (`sequoia-openpgp`, `crypto-rust` on Android).** More complete, more maintained, RFC 9580 first-class, with a higher-level policy API in-box. Rejected: LGPL-2.0+ static-linking into a Play-distributed APK triggers relinking obligations the Play signing/repackaging model makes a recurring per-release legal burden — a cost the MIT/Apache age stack does not pay. Reconsider only if forced (a required Brainpool curve, an OpenPGP-card flow, or Sequoia's policy layer).
- **gpgme / libgpgme FFI.** Maximally gopass-compatible — it is what gopass uses, via system `gpg`. Rejected: gpgme shells out to a `gpg` binary over libassuan, and Android 10+ forbids executing bundled binaries; the full GnuPG C stack is also multi-day autotools cross-compile pain for a result that cannot run. Desktop-only gpgme would split the codebase across two crypto paths.
- **BouncyCastle / PGPainless via JNI on the Android Kotlin layer.** License-clean (MIT), the most interop-tested OpenPGP, no NDK pain. Rejected as primary: desktop Tauri has no JVM, so the Rust side would still need its own implementation, splitting one gopass-compatibility path across two languages and doubling divergence risk on the shared store format. Kept as a fallback.
