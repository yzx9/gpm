# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Migrated the entire Rust backend library (`rustpass`) from synchronous `std::fs` to `tokio::fs`, eliminating UI freezes during file I/O on Android devices

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

[Unreleased]: https://github.com/yzx9/gpm/compare/v0.3.0...HEAD
[v0.3.0]: https://github.com/yzx9/gpm/compare/v0.2.0...v0.3.0
[v0.2.0]: https://github.com/yzx9/gpm/compare/v0.1.0...v0.2.0
[v0.1.0]: https://github.com/yzx9/gpm/releases/tag/v0.1.0
