# AGENTS.md

gpm is an Android-first, age-only gopass password client built with Tauri v2 + Rust + Vue 3. It works against age-encrypted gopass repositories — clone, list, search, decrypt/copy, create secrets (with templates), and sync over git. No GPG-based secret encryption (age-only), no cloud-hosted sync (sync is git pull/push to your own repo). Commit authenticity verifies BOTH SSH-signed and GPG/OpenPGP-signed commits (see Security Model).

## Commands

```bash
just test              # Run all tests (backend + frontend + plugin)
just lint              # Clippy -D warnings + vue-tsc --noEmit
just fmt               # rustfmt + prettier
just dev               # Desktop dev server with hot reload
just android-debug     # Build debug APK
just android-dev       # Android dev server (requires device/emulator)
just kotlin-check      # Fast Kotlin compile gate (catches Android/Kotlin errors)
```

See [DEVELOPMENT.md](DEVELOPMENT.md) for dev environment setup and known issues.

## Architecture

### Frontend — `src/`

SPA web app with Vue3 + TypeScript.

### Backend — `rustpass/`

The crate implements encryption, decryption, Git operations, and repository file management, with its core functionality encapsulated in a `Store` facade. It is an async-first crate built on `tokio`, using `tokio::fs` for all file I/O, while Git and scrypt operations are wrapped in `spawn_blocking`. At this stage, it supports only age encryption and read-only operations, and does not include write capabilities or any UI/CLI interaction logic.

`rustpass` was designed to be compatible with and conceptually aligned with `gopass`, drawing heavily from its architecture and design principles, while intentionally narrowing its scope in the current implementation phase.

### Tauri app — `src-tauri/`

Async Tauri commands, shared app state (`AppState`), and the entry point (`run()`). `lib.rs` is a thin shell — just
`AppState` + `run()`; every command group lives in its own `pub(crate)` module under `src-tauri/src/`, registered in
`run()`'s `invoke_handler`.

### Tauri Plugins — `tauri-plugin-*/`

Local Tauri plugin crates. Each follows the standard Tauri mobile-plugin layout: Rust in `src/`, and its Android Kotlin in its own `android/` Gradle library module (own namespace + build) under a `xyz.yzx9.gpm.{plugin}` package. Tauri auto-discovers each `android/` dir and wires it into the app's gradle build on `tauri android *` runs.

- `tauri-plugin-safe-area` — provides Android safe-area insets to the WebView via standard plugin IPC + events
- `tauri-plugin-biometric-keystore` — stores the identity passphrase in the Android Keystore (AES/GCM, hardware-backed) and retrieves it through a biometric-gated `BiometricPrompt`
- `tauri-plugin-secure-keystore` — seals the at-rest master key with an auth-free, hardware-backed Android Keystore AES/GCM key (the biometric-keystore sibling, minus the prompt; survives fingerprint changes) and returns it to Rust so resources can be AEAD-encrypted at rest
- `tauri-plugin-file-picker` — opens the Android Storage Access Framework picker and reads the picked file's bytes into Rust (backend-only; desktop falls back to `tauri-plugin-dialog`)

## Security Model

- `copy_password` is the primary operation — password never reaches WebView
- `show_password` is secondary — configurable auto-clear (default 45s) with lifecycle cleanup
- Biometric (keystore) unlock is called from Rust app commands, with the passphrase passed from Kotlin to Rust and never exposed to the WebView.
- `repo.json` and `identity` are encrypted at rest on Android (AES-256-GCM; master key sealed in the auth-free Keystore, injected into `rustpass` as bytes). A read attacker / forensic dump gets ciphertext, and a tampered config fails the AEAD tag. Desktop has no Keystore equivalent, so files stay plaintext there (documented asymmetry). The store assumes no local write attacker; a missing/unsealable key degrades to re-setup.
- age plugin recipients (e.g. age-plugin-yubikey's `age1yubikey1...`) are recognized and can be encrypted to: the age library spawns the user-installed `age-plugin-<name>` subprocess to wrap the file key — desktop only, since Android can't run such a binary. That subprocess is the same trust boundary the `age` CLI and gopass already assume (the user trusts the binary they installed); no secret reaches the WebView, only age file keys/stanzas cross the plugin's stdio protocol. Plugin _identities_ (decrypting with a hardware key) are recognized but not yet supported. A missing binary surfaces as a clear `PluginUnavailable` error instead of a silent write failure.
- All decrypted content uses `Zeroizing<String>` and is wiped after use
- Error messages are sanitized to never contain secrets
- CSP restricts script/connect sources to `self` + IPC only
- Auto-lock defaults to "Immediate" (no-cache): the identity is decrypted per copy/show/create and wiped right after, so the master key sits in memory only for the operation, not the whole session. Browsing the list needs no identity. The identity cache is also wiped on a failed op (a decode error under Immediate still clears the cache). Writes are local-only, then published by the autosync orchestrator (pull → write → push); there is no conflict stash, so the Immediate wipe always proceeds — except on a `NeedsDivergenceResolve` outcome, where the wipe is deferred so a keep-mine resolve can reuse the cached identity without a second unlock; that deferred wipe runs both in the resolve step and on resolve-cancel (`discard_divergence`), so abandoning the modal never strands the key. Idle-timeout and Never modes keep the session cached as before.
- AutoSync (per-device, on by default): when on, every save pull-write-pushes automatically; when off, saves are local-only until a manual Sync (pull + push) publishes them. The divergence resolve prompt catches only the push-rejection race (a save that directly collides with a newer remote); a save built on an out-of-date read can still fast-forward over and silently overwrite a newer remote change — recoverable in git history, surfaced as a note under the AutoSync setting (RFC 0026 is the base-version-aware fix).

See [SECURITY.md](docs/SECURITY.md) for the full threat model and known limitations.

## Testing

Backend tests are in-module (`#[cfg(test)]` next to the code) plus integration tests in `rustpass/tests/` (store facade, config persistence, crypto). Frontend tests are vitest in `src/**/*.test.ts` (mocked `@tauri-apps/api/core` `invoke`). There is no `src-tauri/tests/` directory. When changing Kotlin — app code under `src-tauri/gen/android/app/` or a plugin's `android/src/main/java/` — run `just kotlin-check` before finishing — it compiles the app's Kotlin in seconds and catches errors that otherwise only surface inside the multi-minute `tauri android build`.

The local Android plugins' Robolectric/JVM unit tests run via `just test-plugin` (→ `./gradlew testPlugins`, which fans out across every local plugin's `testDebugUnitTest`). The gate is gated on `src-tauri/gen/android/tauri.settings.gradle` — gitignored, generated by `tauri android build/dev` — so run `just android-debug` once to materialize it. The recipe fails loud if the file is stale (wrong repo root) or omits any local path plugin from `src-tauri/Cargo.toml`, since both would silently skip a plugin's tests.

## Conventions

- **gopass compatibility is a hard constraint.** gpm's templates, presets, and secret formats mirror gopass's on-disk/semantic formats — do not invent a parallel abstraction when gopass already defines the concept. Example: the create-wizard field model mirrors gopass's `Attribute` (`type`/`charset`/`min`/`max`/`strict`); PIN vs password is distinguished by per-attribute `charset` (PIN = `0123456789`), not a custom flag. When adding a feature gopass has, check gopass's source (`pkg/pwgen`, `internal/create`, …) and match its schema/semantics.
- SPDX license headers on all source files
- Nix flake provides the full dev environment (`direnv allow` to activate)
- Single age identity only (multi-identity deferred); supports x25519 native keys (optionally passphrase-encrypted at rest) and SSH private keys (ed25519, RSA), including passphrase-protected SSH keys
- HTTPS and SSH Git remotes (SSH key generation + paste)
- Biometric unlock (fingerprint/face) on Android 11+ for passphrase-protected identities. Desktop and Android <11 stay passphrase-only. iOS deferred.
- `gen/android/` looks like a generated directory but contains git-tracked, manually maintained files — **except `app/build.gradle.kts`, which `tauri android build` re-renders from its template every run, silently dropping manual edits.** Put manual gradle config (e.g. `applicationIdSuffix`) in `gen/android/settings.gradle` instead, which Tauri does not regenerate.
- Tauri v2 IPC naming: Rust uses `snake_case`, frontend/Kotlin use `camelCase` — Tauri auto-converts at the boundary. Match the existing plugin code.
- The Android debug build sets `applicationIdSuffix = ".debug"` (installs as `xyz.yzx9.gpm.debug`) so it coexists with the release — install a debug build for diagnostics without uninstalling.
- Update `CHANGELOG.md` when adding user-facing changes. Keep entries user-focused (no technical internals).

## Design RFCs

`docs/rfcs` holds lightweight design RFCs. It is the parking lot for work that is deliberately out of the current PR or phase: ideas discovered during implementation, deferred scope, and larger future improvements. An RFC captures the **problem, the design decision, and the rationale** — not the implementation.

Write an RFC when:

- A decision is non-obvious, reversible only with effort, or touches the architecture or threat model.
- A thought came up during implementation but does not belong in the current PR.
- A phase just landed and you want to record the next, larger improvement.

When writing one:

- **Read `0000-rfc-template.md` first, before writing anything.** It is the spec: the header metadata, the section structure, the file-naming / numbering rules, and the altitude rule all live there.

RFC references and lifecycle:

- Do not reference temporary RFCs, review labels, or planning artifacts in code, docs, comments, or commit messages; instead, write self-contained explanations of the what and why.
- If an RFC is completed or superseded, it may be removed.

## Compact Instructions

When compressing, preserve in priority order:

1. Architecture decisions (NEVER summarize)
2. Modified files and their key changes
3. Current verification status (pass/fail)
4. Open TODOs and rollback notes
5. Tool outputs (can delete, keep pass/fail only)
