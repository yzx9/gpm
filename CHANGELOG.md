# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- The backend can now write a new secret the way gopass does: it syncs first, encrypts the content to every store recipient (always including your own key, so you can read back what you wrote), saves it at the chosen path, and commits and pushes the change. This is the foundation for in-app secret creation
- When a write collides with a newer remote copy of the same secret (e.g. you wrote offline and the remote moved), the backend detects the conflict instead of failing blindly. It reports whether the remote copy is one you can decrypt, and lets the caller resolve it: keep your version, keep the remote's, back out, or (with explicit confirmation) force your version over one you can't read. The conflict result never contains any plaintext, so the choice stays safe to pass to the UI
- The backend understands gopass content templates and creation presets. A `.pass-template` placed in a store directory now shapes any new secret created beneath it (filling in the password and layout), and the built-in "Website login" and "PIN Code" presets generate a secret at a fixed location (under `websites/` or `pin/`) from a few fields — the same "create" flow gopass offers
- Create new secrets right from the app: pick a Website login, PIN code, or a custom name, and gpm encrypts and pushes it just like gopass. A `.pass-template` in a folder automatically shapes any new secret created beneath it, and you can preview the result before saving
- If a new secret collides with a newer remote copy, the app asks how to resolve it instead of failing — keep yours, keep the existing one, or cancel. When the existing copy is one you can read, you can preview it first; overwriting one you can't read is blocked behind an explicit confirmation so you can't unknowingly destroy it

### Changed

- When gpm auto-locks after 5 minutes (or on launch of a passphrase-protected identity), the unlock prompt now appears as an overlay over whatever screen you were on, and unlocking drops you back exactly where you were — your scroll position and current entry are preserved. The biometric auto-prompt, cancel, and reset handling moved into the overlay unchanged

### Fixed

- The instant the identity locks, every currently-revealed secret across the app is cleared — a shown password, an exported SSH key, a half-typed new secret. Previously the old unlock redirect gave this for free by unmounting the page; the new overlay keeps pages mounted, so clear-on-lock is now explicit
- A stale auto-lock timer could re-lock the app moments after a fresh unlock; the timer now carries a monotonic generation tag and disarms itself if a newer unlock happened while it slept

## [v0.5.0] - 2026-06-15

### Added

- Upload an identity file instead of pasting it during setup. The file is opened, read, and parsed entirely on-device by the backend; its contents never reach the app UI. Encrypted files (a passphrase-protected SSH key, or an age-encrypted identity) prompt for the passphrase immediately and discard the file on a wrong one; once usable, the derived public key is shown so you can confirm it matches a recipient. Files produced by `age-keygen` (with `#` comment lines) are also supported
- Optional repository authenticity verification: detect a compromised git remote feeding validly encrypted but wrong entries by verifying the SSH signature on every commit pulled. A new tri-state setting (Off / Audit / Enforce) controls behaviour — Audit warns on a mismatch but always pulls, Enforce blocks the pull when a commit is unsigned, untrusted, or tampered, leaving your store on the last verified state. Manage trusted signing keys in Settings and review per-commit signature status in the new History screen. Off by default; nothing changes until you enable it

### Fixed

- Use plain val for Charset constant in KeystorePlugin

## [v0.4.0] - 2026-06-14

### Added

- Biometric unlock (fingerprint or face) for passphrase-protected identities on Android 11 and above — unlock gpm with biometrics instead of typing your passphrase on every launch. The passphrase is sealed in the Android Keystore with hardware-backed, biometric-gated encryption, and works for both age and SSH identities that have a passphrase. Enabling or changing your passphrase invalidates biometric unlock and asks you to re-enable it. Desktop and Android below 11 keep the passphrase-only flow

### Changed

- Migrated the entire Rust backend library (`rustpass`) from synchronous `std::fs` to `tokio::fs`, eliminating UI freezes during file I/O on Android devices
- Post-quantum (X-Wing) age keys are now recognized and show a clear "not yet supported" message during setup and decryption, instead of failing with a confusing error. Post-quantum recipients in the repository are also labeled accurately in the setup wizard rather than appearing as ordinary age keys

### Removed

- SSH key identities are no longer re-encrypted by gpm; they rely on their own native passphrase protection, matching how age handles them. The setup wizard now uses a single passphrase field (for x25519 at-rest encryption or SSH key decryption, depending on the identity type) instead of two separate fields

## [v0.3.0] - 2026-06-12

### Added

- Optional passphrase to encrypt identity at rest (setup wizard or settings)
- Unlock screen when identity is passphrase-encrypted
- Auto-lock after 5 minutes of inactivity
- Passphrase management in settings: set, change, or remove
- SSH key authentication for Git operations (`git@host:repo` and `ssh://` URLs)
- Passphrase-encrypted SSH private keys as age identities (passphrase prompted during setup, cached at runtime)

## [v0.2.0] - 2026-06-10

### Added

- On-device ed25519 SSH key generation with optional passphrase
- Settings page with public key display and private key export
- Two-step setup wizard: clone repo first, then select a recipient and provide matching age identity
- Recipient discovery from `.gopass-recipients` / `.age-recipients` files in cloned repositories
- Identity validation on setup: derived public key is checked against known recipients
- SSH key recipient support: decrypt entries encrypted to `ssh-ed25519` or `ssh-rsa` public keys using the corresponding SSH private key as identity
- Recipient type detection (x25519, SSH ed25519, SSH RSA) with SSH badge in setup wizard
- SSH key reuse: one-click "Use my SSH key for decryption" when Git auth and age recipient use the same key

## [v0.1.0] - 2026-06-08

In this initial release, we have implement a read-only age-only gopass password client for Android, built with Tauri v2 + Rust + Vue 3.

### Added

- Clone age-encrypted gopass repositories via HTTPS + PAT
- List and search password entries
- Decrypt and copy passwords to clipboard
- Show password with 30-second auto-clear and lifecycle cleanup
- Pull-to-refresh to sync with remote repository
- Android APK signing and per-architecture release builds

[Unreleased]: https://github.com/yzx9/gpm/compare/v0.5.0...HEAD
[v0.5.0]: https://github.com/yzx9/gpm/compare/v0.4.0...v0.5.0
[v0.4.0]: https://github.com/yzx9/gpm/compare/v0.3.0...v0.4.0
[v0.3.0]: https://github.com/yzx9/gpm/compare/v0.2.0...v0.3.0
[v0.2.0]: https://github.com/yzx9/gpm/compare/v0.1.0...v0.2.0
[v0.1.0]: https://github.com/yzx9/gpm/releases/tag/v0.1.0
