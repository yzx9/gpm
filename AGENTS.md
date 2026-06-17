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
just kotlin-check      # Fast Kotlin compile gate (catches Android/Kotlin errors before the full build)
```

See [DEVELOPMENT.md](DEVELOPMENT.md) for dev environment setup and known issues.

## Architecture

### Frontend — `src/`

SPA web app with Vue3 + TypeScript. All Tauri IPC types live in `src/types.ts`.

### Backend — `rustpass/`

The crate implements encryption, decryption, Git operations, and repository file management, with its core functionality encapsulated in a `Store` facade. It is an async-first crate built on `tokio`, using `tokio::fs` for all file I/O, while Git and scrypt operations are wrapped in `spawn_blocking`. At this stage, it supports only age encryption and read-only operations, and does not include write capabilities or any UI/CLI interaction logic.

`rustpass` was designed to be compatible with and conceptually aligned with `gopass`, drawing heavily from its architecture and design principles, while intentionally narrowing its scope in the current implementation phase.

### Tauri app — `src-tauri/`

Async Tauri commands, shared app state (`AppState`), and the entry point (`run()`). `lib.rs` is a thin shell — just
`AppState` + `run()`; every command group lives in its own `pub(crate)` module under `src-tauri/src/`, registered in
`run()`'s `invoke_handler`.

### Tauri Plugins — `tauri-plugin-*/`

Local Tauri plugin crates (not published). Each follows the standard Tauri mobile-plugin layout: Rust in `src/`, and its Android Kotlin in its own `android/` Gradle library module (own namespace + build) under a `xyz.yzx9.gpm.{plugin}` package. Tauri auto-discovers each `android/` dir and wires it into the app's gradle build on `tauri android *` runs.

- `tauri-plugin-safe-area` — provides Android safe-area insets to the WebView via standard plugin IPC + events
- `tauri-plugin-biometric-keystore` — stores the identity passphrase in the Android Keystore (AES/GCM, hardware-backed) and retrieves it through a biometric-gated `BiometricPrompt`
- `tauri-plugin-file-picker` — opens the Android Storage Access Framework picker and reads the picked file's bytes into Rust (backend-only; desktop falls back to `tauri-plugin-dialog`)

## Security Model

- `copy_password` is the primary operation — password never reaches WebView
- `show_password` is secondary — 30s auto-clear with lifecycle cleanup
- Biometric (keystore) unlock is called from Rust app commands, with the passphrase passed from Kotlin to Rust and never exposed to the WebView.
- All decrypted content uses `Zeroizing<String>` and is wiped after use
- Error messages are sanitized to never contain secrets
- CSP restricts script/connect sources to `self` + IPC only

See [SECURITY.md](SECURITY.md) for the full threat model and known limitations.

## Testing

Backend tests are in-module (`#[cfg(test)]` next to the code) plus integration tests in `rustpass/tests/` (store facade, config persistence, crypto). Frontend tests are vitest in `src/**/*.test.ts` (mocked `@tauri-apps/api/core` `invoke`). There is no `src-tauri/tests/` directory. When changing Kotlin — app code under `src-tauri/gen/android/app/` or a plugin's `android/src/main/java/` — run `just kotlin-check` before finishing — it compiles the app's Kotlin in seconds and catches errors that otherwise only surface inside the multi-minute `tauri android build`.

## Conventions

- Update `CHANGELOG.md` when adding user-facing changes. Keep entries user-focused (no technical internals).
- SPDX license headers on all source files
- Nix flake provides the full dev environment (`direnv allow` to activate)
- Single age identity only (multi-identity deferred); supports x25519 native keys (optionally passphrase-encrypted at rest) and SSH private keys (ed25519, RSA), including passphrase-protected SSH keys
- HTTPS and SSH Git remotes (SSH key generation + paste)
- Biometric unlock (fingerprint/face) on Android 11+ for passphrase-protected identities (age or SSH); the passphrase is sealed in the Android Keystore with hardware-backed, biometric-gated encryption. Desktop and Android <11 stay passphrase-only. iOS deferred.
- `src-tauri/gen/android/` looks like a generated directory but contains git-tracked, manually maintained files (e.g. `MainActivity.kt`, `AndroidManifest.xml`, resources, the app `build.gradle.kts`). Plugin Kotlin lives in each plugin crate's own `android/` module, not here. Do not assume `gen/android/` contents are auto-generated or disposable.

## Design RFCs — `.plans/`

`.plans/` holds lightweight design RFCs. It is the parking lot for work that is deliberately out of the current PR or phase: ideas discovered during implementation, deferred scope, and larger future improvements. An RFC captures the **problem, the design decision, and the rationale** — not the implementation.

Write an RFC when:

- A decision is non-obvious, reversible only with effort, or touches the architecture or threat model.
- A thought came up during implementation but does not belong in the current PR.
- A phase just landed and you want to record the next, larger improvement.

### How to write one

- One file per RFC: `NNNN-kebab-title.md`.
- `NNNN` is 4-digit zero-padded; **next number = current max + 1**
- Follow the template in `0000-rfc-template.md`.
- If an RFC is completed or superseded, it may be removed.

### Altitude — the one rule

If you are writing file paths, line numbers, struct fields, function signatures, or code, you have dropped below RFC altitude — move it into the implementation. An RFC should still read cleanly after the code it describes has been rewritten twice. The RFC records _why_; the implementation records _how_.

## Compact Instructions

When compressing, preserve in priority order:

1. Architecture decisions (NEVER summarize)
2. Modified files and their key changes
3. Current verification status (pass/fail)
4. Open TODOs and rollback notes
5. Tool outputs (can delete, keep pass/fail only)
