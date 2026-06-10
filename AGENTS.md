# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

gpm is an Android-first, age-only gopass password client built with Tauri v2 + Rust + Vue 3. It provides a read-only GUI for age-encrypted gopass repositories (clone, list, search, decrypt, copy). No GPG, no editing, no cloud sync.

## Commands

```bash
just test              # Run all tests (backend + frontend)
just lint              # Clippy -D warnings + vue-tsc --noEmit
just fmt               # rustfmt + prettier
just dev               # Desktop dev server with hot reload
just android-debug     # Build debug APK
just android-release   # Build release APK (signed if keystore.properties exists)
just android-dev       # Android dev server (requires device/emulator)
just android-install   # Build + install debug APK to connected device
just android-install-release # Build + install release APK to connected device
```

See [DEVELOPMENT.md](DEVELOPMENT.md) for dev environment setup and known issues.

## Architecture

### Frontend (Vue 3 + TypeScript) — `src/`

Four-page SPA with Vue Router:

- **SetupPage** — Git URL + auth (PAT/SSH key) + age identity → clone repo
- **EntryListPage** — List/search entries, copy passwords, pull to refresh
- **EntryDetailPage** — Show password with 30s auto-clear
- **SettingsPage** — View public key, export private key, reset

All Tauri IPC types live in `src/types.ts`.

### Backend (Rust) — `src-tauri/src/`

- `lib.rs` — Tauri commands, state management, app entry
- `crypto.rs` — Age decryption with zeroize-per-decrypt
- `ssh.rs` — SSH key generation (ed25519), public key derivation, private key export
- `store.rs` — Directory walking, .age file discovery, content parsing
- `git.rs` — Clone + pull (ff-only) via git2
- `secure_storage.rs` — Identity + config persistence
- `error.rs` — Safe error types (no secrets in messages)

### Tauri Plugins — `gpm-plugin-safe-area/`

Local Tauri plugin crate (not published). Provides Android safe-area insets to the frontend via standard plugin IPC + event system.

### Security Model

- `copy_password` is the primary operation — password never reaches WebView
- `show_password` is secondary — 30s auto-clear with lifecycle cleanup
- All decrypted content uses `Zeroizing<String>` and is wiped after use
- Error messages are sanitized to never contain secrets
- CSP restricts script/connect sources to `self` + IPC only

See [SECURITY.md](SECURITY.md) for the full threat model and known limitations.

## Testing

Integration tests in `src-tauri/tests/fixtures.rs` covering store parsing, content parsing, crypto (correct/wrong identity, corrupted data), and security (no secrets in errors).

## Conventions

- Rust lint config in `lib.rs` has extensive `#![warn(...)]` attributes — Clippy warnings are errors
- SPDX license headers on all source files
- Nix flake provides the full dev environment (`direnv allow` to activate)
- Single age identity only (multi-identity deferred)
- HTTPS and SSH Git remotes (SSH key generation + paste)
