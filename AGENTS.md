# CLAUDE.md

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
```

See [DEVELOPMENT.md](DEVELOPMENT.md) for dev environment setup and known issues.

## Architecture

### Frontend — `src/`

SPA web app with Vue3 + TypeScript. All Tauri IPC types live in `src/types.ts`.

### Backend — `rustpass/`

The crate implements encryption, decryption, Git operations, and repository file management, with its core functionality encapsulated in a `Store` facade. It is an async-first crate built on `tokio`, using `tokio::fs` for all file I/O, while Git and scrypt operations are wrapped in `spawn_blocking`. At this stage, it supports only age encryption and read-only operations, and does not include write capabilities or any UI/CLI interaction logic.

`rustpass` was designed to be compatible with and conceptually aligned with `gopass`, drawing heavily from its architecture and design principles, while intentionally narrowing its scope in the current implementation phase.

### Tauri app — `src-tauri/`

Async Tauri commands, state management, app entry, IPC types. Includes the biometric commands (`is_biometric_available`, `enable_biometric_unlock`, `biometric_unlock`, …) backed by the keystore plugin, plus the shared `unlock_and_arm` helper used by both the password and biometric unlock paths

### Tauri Plugins — `gpm-plugin-safe-area/`, `gpm-plugin-keystore/`

Local Tauri plugin crates (not published):

- `gpm-plugin-safe-area` — provides Android safe-area insets to the WebView via standard plugin IPC + events
- `gpm-plugin-keystore` — stores the identity passphrase in the Android Keystore (AES/GCM, hardware-backed) and retrieves it through a biometric-gated `BiometricPrompt`

## Security Model

- `copy_password` is the primary operation — password never reaches WebView
- `show_password` is secondary — 30s auto-clear with lifecycle cleanup
- Biometric (keystore) unlock is called from Rust app commands, with the passphrase passed from Kotlin to Rust and never exposed to the WebView.
- All decrypted content uses `Zeroizing<String>` and is wiped after use
- Error messages are sanitized to never contain secrets
- CSP restricts script/connect sources to `self` + IPC only

See [SECURITY.md](SECURITY.md) for the full threat model and known limitations.

## Testing

Backend tests are in-module (`#[cfg(test)]` next to the code) plus integration tests in `rustpass/tests/` (store facade, config persistence, crypto). Frontend tests are vitest in `src/**/*.test.ts` (mocked `@tauri-apps/api/core` `invoke`). There is no `src-tauri/tests/` directory.

## Conventions

- Update `CHANGELOG.md` when adding user-facing changes. Keep entries user-focused (no technical internals).
- SPDX license headers on all source files
- Nix flake provides the full dev environment (`direnv allow` to activate)
- Single age identity only (multi-identity deferred); supports x25519 native keys (optionally passphrase-encrypted at rest) and SSH private keys (ed25519, RSA), including passphrase-protected SSH keys
- HTTPS and SSH Git remotes (SSH key generation + paste)
- Biometric unlock (fingerprint/face) on Android 11+ for passphrase-protected identities (age or SSH); the passphrase is sealed in the Android Keystore with hardware-backed, biometric-gated encryption. Desktop and Android <11 stay passphrase-only. iOS deferred.
- `src-tauri/gen/android/` looks like a generated directory but contains git-tracked, manually maintained files (e.g. `SafeAreaPlugin.kt`, `KeystorePlugin.kt`). Do not assume its contents are auto-generated or disposable.

## Compact Instructions

When compressing, preserve in priority order:

1. Architecture decisions (NEVER summarize)
2. Modified files and their key changes
3. Current verification status (pass/fail)
4. Open TODOs and rollback notes
5. Tool outputs (can delete, keep pass/fail only)
