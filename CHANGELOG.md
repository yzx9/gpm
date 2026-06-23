# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Start a brand-new password store right on this device — no existing repo and no second tool required. Setup now offers "Create a new store" alongside "Clone": generate an age or SSH identity in-app, seed the store, and optionally add a git remote to sync later. A store gpm creates is indistinguishable from one gopass creates, so you can mix tools or migrate freely
- Control how and when gpm locks, all from the new "Auto-Lock & Auto-Clear" section in Settings. Pick when the app locks (immediately after each action, after a few minutes idle, or never), how long a shown password stays on screen, and how long the clipboard holds a copy — each with sensible presets and a "Never" option

### Changed

- gpm now defaults to re-checking your fingerprint or passphrase each time you copy, view, or create a secret, rather than staying unlocked for minutes at a time. This keeps your decryption key in memory only for the instant it's needed. Browsing the list is unaffected (it never needs unlocking). If you prefer the old "stay unlocked for a while" behavior, switch Auto-Lock to an idle timeout in Settings
- A shown password now auto-clears after 45 seconds by default (was 30), and a copied password clears from the clipboard after 45 seconds by default (was 30) — both are now adjustable in Settings

## [v0.6.0] - 2026-06-20

### Added

- On Android, gpm now encrypts your local configuration and identity at rest with a key sealed in the device's hardware-backed Keystore, so someone who copies the app's private files (a stolen backup, a forensic dump) gets ciphertext rather than your git credentials or decryption key. Existing data is wrapped automatically on the next launch, and tampering with these files is detected and rejected. Desktop is unchanged
- The backend can now write a new secret the way gopass does: it syncs first, encrypts the content to every store recipient (always including your own key, so you can read back what you wrote), saves it at the chosen path, and commits and pushes the change. This is the foundation for in-app secret creation
- When a write collides with a newer remote copy of the same secret (e.g. you wrote offline and the remote moved), the backend detects the conflict instead of failing blindly. It reports whether the remote copy is one you can decrypt, and lets the caller resolve it: keep your version, keep the remote's, back out, or (with explicit confirmation) force your version over one you can't read. The conflict result never contains any plaintext, so the choice stays safe to pass to the UI
- The backend understands gopass content templates and creation presets. A `.pass-template` placed in a store directory now shapes any new secret created beneath it (filling in the password and layout), and the built-in "Website login" and "PIN Code" presets generate a secret at a fixed location (under `websites/` or `pin/`) from a few fields — the same "create" flow gopass offers
- Create new secrets right from the app: pick a Website login, PIN code, or a custom name, and gpm encrypts and pushes it just like gopass. A `.pass-template` in a folder automatically shapes any new secret created beneath it, and you can preview the result before saving
- If a new secret collides with a newer remote copy, the app asks how to resolve it instead of failing — keep yours, keep the existing one, or cancel. When the existing copy is one you can read, you can preview it first; overwriting one you can't read is blocked behind an explicit confirmation so you can't unknowingly destroy it
- When a pull finds the local and remote password stores have diverged, the app lists the local-only and modified secrets (and other files) that would be lost and offers to adopt the remote, discarding those local changes behind an explicit confirmation — instead of failing with an unresolvable error
- You can now choose the name and email gpm writes on each git commit — set it under Advanced Settings during setup, or change it later in Settings. Leave it blank to keep the built-in default, which follows app updates until you pick your own

### Changed

- The entry list and search now load one page at a time instead of pulling every entry into the app at once — as you scroll, more entries load automatically, with a "Load more" button as a fallback. This keeps the list fast and light on memory as your store grows, and search results page the same way
- Searching entries is now fuzzy: type a few letters in order (like `awroot`) to jump to `cloud/aws/root`, matching anywhere in the name or path. Search also runs in the backend now, so it stays fast as the store grows and keeps working when the list later loads on demand
- When gpm auto-locks after 5 minutes (or on launch of a passphrase-protected identity), the unlock prompt now appears as an overlay over whatever screen you were on, and unlocking drops you back exactly where you were — your scroll position and current entry are preserved. The biometric auto-prompt, cancel, and reset handling moved into the overlay unchanged
- Unlocking with an SSH key is faster: the key is decrypted once when you unlock, so opening each secret afterwards is quicker instead of paying that cost on every copy or show. The unlock passphrase is also no longer held in memory for the whole session — it's used to decrypt your identity and then dropped

### Fixed

- The instant the identity locks, every currently-revealed secret across the app is cleared — a shown password, an exported SSH key, a half-typed new secret. Previously the old unlock redirect gave this for free by unmounting the page; the new overlay keeps pages mounted, so clear-on-lock is now explicit
- A stale auto-lock timer could re-lock the app moments after a fresh unlock; the timer now carries a monotonic generation tag and disarms itself if a newer unlock happened while it slept

### Security

- Enabling biometric unlock is now refused in the backend for identities that have no passphrase, instead of relying on the settings screen to hide the option — a defense-in-depth backstop in case that UI gate ever regresses

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

[Unreleased]: https://github.com/yzx9/gpm/compare/v0.6.0...HEAD
[v0.6.0]: https://github.com/yzx9/gpm/compare/v0.5.0...v0.6.0
[v0.5.0]: https://github.com/yzx9/gpm/compare/v0.4.0...v0.5.0
[v0.4.0]: https://github.com/yzx9/gpm/compare/v0.3.0...v0.4.0
[v0.3.0]: https://github.com/yzx9/gpm/compare/v0.2.0...v0.3.0
[v0.2.0]: https://github.com/yzx9/gpm/compare/v0.1.0...v0.2.0
[v0.1.0]: https://github.com/yzx9/gpm/releases/tag/v0.1.0
