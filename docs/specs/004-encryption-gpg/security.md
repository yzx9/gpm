<!--
Feature-level threat model for the GPG/OpenPGP crypto backend. Forward-looking — the
backend is implemented and seam-tested but NOT yet wired into the Store (prd.md §6).
Complements docs/SECURITY.md. Living.
-->

# 004 — GPG encryption: threat model

## Assets & trust boundary

The GPG keyring's secret keys (located by fingerprint) — whoever holds them decrypts a
gopass-GPG store. Trust boundaries: the rpgp library (in-process, pure Rust) and process
memory. No system `~/.gnupg` — the keyring lives in-app.

## rpgp panic isolation

rpgp is treated as an untrusted parser: all rpgp calls are wrapped in `catch_unwind`, so
a panic in OpenPGP parsing cannot take down the main process. A panic surfaces as a
sanitized error, never a crash leaking secret context.

## S2K passphrase unlock

The GPG secret key is protected by its S2K passphrase; unlock reuses the existing
biometric-keystore + AutoLock machinery (`007/security.md`) — the passphrase is entered
once, gated by biometrics, and cached per the same AutoLock lifecycle as the age
identity. Brute-force resistance is S2K's responsibility (gpg's standard).

## In-app keyring, no system gpg

No `gpgme`, no JNI, no shelling out to a system `gpg` — so no system-binary trust vector.
The keyring is application state; trusted signing keys are managed alongside 005's
authenticity trust set.

## Recognized-but-unsupported, honest errors

OpenPGP-card, Brainpool curves, and other unsupported variants are detected and surface
a clear "not supported" status (the same pattern as 003's PQ/plugin handling) rather
than a silent failure. Errors are sanitized (system-wide rule).

## Open (until wired in)

- Whether `BackendKind` persists in `repo.json` and how crypto-backend selection works
  (typed selection vs a registry) — see `prd.md` §5.
- GPG setup sub-flow and keyring-management UI threat surface — defined when built.

## Cross-references

- rpgp choice rationale: `prd.md` §5; ADR (rpgp) TBD.
- Shared rpgp seam with 005's signature verification.
