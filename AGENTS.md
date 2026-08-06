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
```

See [DEVELOPMENT.md](DEVELOPMENT.md) for dev environment setup and known issues.

## Architecture

### Frontend — `app/src/`

SPA web app with Vue3 + TypeScript.

### Backend — `crates/rustpass/`

The crate implements encryption, decryption, Git operations, and repository file management, with its core functionality encapsulated in a `Store` facade. It is an async-first crate built on `tokio`, using `tokio::fs` for all file I/O, while Git and scrypt operations are wrapped in `spawn_blocking`. It is a library crate only — no UI or CLI; the Tauri app (`app/src-tauri/`) exposes its `Store` as commands.

`rustpass` was designed to be compatible with and conceptually aligned with `gopass`, drawing heavily from its architecture and design principles, while intentionally narrowing its scope in the current implementation phase.

### Tauri app — `app/src-tauri/`

Async Tauri commands, shared app state (`AppState`), and the entry point (`run()`). `lib.rs` is a thin shell — just
`AppState` + `run()`; every command group lives in its own `pub(crate)` module under `app/src-tauri/src/`, registered in
`run()`'s `invoke_handler`.

### Tauri Plugins — `crates/tauri-plugin-*/`

Local Tauri plugin crates. Each follows the standard Tauri mobile-plugin layout: Rust in `src/`, and its Android Kotlin in its own `android/` Gradle library module (own namespace + build) under a `xyz.yzx9.gpm.{plugin}` package. Tauri auto-discovers each `android/` dir and wires it into the app's gradle build on `tauri android *` runs.

A plugin crate exists to be **publishable as a standalone, app-agnostic primitive**; anything tightly coupled to gpm (key aliases, business logic, store internals) belongs in the app, not a plugin. The plugins below carry no gpm identifiers for this reason.

- `tauri-plugin-safe-area` — provides Android safe-area insets to the WebView via standard plugin IPC + events
- `tauri-plugin-keystore` — a **generic** Android Keystore seal for a caller-supplied secret string, under a caller-chosen policy (auth-free, or biometric-gated via a per-use `BiometricPrompt`); hardware-backed AES/GCM. `alias`/`prefs`/`policy`/`prompt` are all caller-supplied — the plugin carries no gpm identifiers or brand strings (the app passes them from `keystore.rs`). gpm uses it for the identity passphrase (biometric-gated), the at-rest master key (auth-free), and the App Lock vault key (biometric-gated)
- `tauri-plugin-file-picker` — opens the Android Storage Access Framework picker and reads the picked file's bytes into Rust (backend-only; desktop falls back to `tauri-plugin-dialog`)
- `tauri-plugin-file-save` — saves a staged file to a user-picked destination via Android SAF `ACTION_CREATE_DOCUMENT`, owning the Kotlin write for a real error path (backend-only; desktop falls back to `tauri-plugin-dialog`'s save)
- `tauri-plugin-screen-secure` — toggles Android `FLAG_SECURE` for per-route screen-capture protection on sensitive screens (frontend calls `set_secure(bool)`; desktop no-op, gated by `screen_secure_available()`)
- `tauri-plugin-clipboard-notify` — a **generic** sticky-notification + tap-to-clear: posts a sticky Android notification with caller-resolved text while something is on the clipboard; the tap clears natively and sets a manual-clear flag the Rust clear timer polls (no Kotlin→Rust event — the flag is polled over `run_mobile_plugin_async`). Backend-only; inert no-ops on desktop. gpm uses it for the clipboard-clear notification
- `tauri-plugin-device-info` — surfaces Android hardware/OS build fields, the WebView user-agent, and display metrics to Rust for the diagnostics export (backend-only; desktop gets a minimal OS/arch/version fallback)
- `tauri-plugin-background-sync` — schedules the periodic Android background git sync via `WorkManager` (network-gated) and cancels it when the cadence is `Off` (backend-only; inert no-ops on desktop, where the foreground sync covers it)

## Security Model

- `copy_password` is the primary operation — password never reaches WebView
- `show_password` is secondary — configurable auto-clear (default 45s) with lifecycle cleanup
- Biometric (keystore) unlock is called from Rust app commands, with the passphrase passed from Kotlin to Rust and never exposed to the WebView.
- `repo.json`, the app config (`app.json`), and `identity` are encrypted at rest on Android (AES-256-GCM; master key sealed in the auth-free Keystore). The app config holds zero plaintext — display prefs (locale/theme) and behavior prefs are sealed together. A read attacker / forensic dump gets ciphertext, and a tampered config fails the AEAD tag. Desktop has no Keystore equivalent, so files stay plaintext there. The store assumes no local write attacker; a missing/unsealable key degrades to re-setup.
- age plugin recipients (e.g. age-plugin-yubikey's `age1yubikey1...`) are recognized and can be encrypted to: the age library spawns the user-installed `age-plugin-<name>` subprocess to wrap the file key — desktop only, since Android can't run such a binary. That subprocess is the same trust boundary the `age` CLI and gopass already assume; no secret reaches the WebView, only age file keys/stanzas cross the plugin's stdio protocol. Plugin _identities_ (decrypting with a hardware key) are recognized but not yet supported. A missing binary surfaces as a clear `PluginUnavailable` error instead of a silent write failure.
- All decrypted content uses `Zeroizing<String>` and is wiped after use
- Error messages are sanitized to never contain secrets
- CSP restricts script/connect sources to `self` + IPC only
- Auto-lock: the identity is decrypted per copy/show/create and wiped right after, so the master key sits in memory only for the operation, not the whole session. Browsing the list needs no identity. The identity cache is also wiped on a failed op. Writes are local-only, then published by the autosync orchestrator (pull → write → push); there is no conflict stash, so the Immediate wipe always proceeds — except on a `NeedsDivergenceResolve` outcome, where the wipe is deferred so a keep-mine resolve can reuse the cached identity without a second unlock; that deferred wipe runs both in the resolve step and on resolve-cancel (`discard_divergence`), so abandoning the modal never strands the key. Idle-timeout and Never modes keep the session cached as before. Under Idle the timer also resets on in-app activity, not just secret operations.
- AutoSync: when on, every save pull-write-pushes automatically; when off, saves are local-only until a manual Sync (pull + push) publishes them. The divergence resolve prompt catches only the push-rejection race (a save that directly collides with a newer remote); a save built on an out-of-date read can still fast-forward over and silently overwrite a newer remote change — recoverable in git history, surfaced as a note under the AutoSync setting.

See [SECURITY.md](docs/SECURITY.md) for the full threat model and known limitations.

## Testing

Backend tests are in-module plus integration tests. Frontend tests are vitest in `app/src/**/*.test.ts`. The fast local gates have a blind spot: `just test` and `just lint` compile on the host and never build `#[cfg(target_os = "android")]` code, and `just test-plugin` runs the plugins' JVM tests without compiling the app's Kotlin — so a green fast gate does not mean the Android build is green. The full `tauri android build` is slow (minutes) — decide whether your change needs it, but run it (or the quicker `cargo check --target aarch64-linux-android`) when a change touches android-gated code (a JNI shim, a plugin's `android/` code, Kotlin) rather than skipping it on the strength of the fast gates.

The local Android plugins' Robolectric/JVM unit tests run via `just test-plugin`. The gate is gated on `app/src-tauri/gen/android/tauri.settings.gradle` — gitignored, generated by `tauri android build/dev` — so run `just android-debug` once to materialize it.

## Conventions

- **gopass compatibility is a hard constraint.** gpm's templates, presets, and secret formats mirror gopass's on-disk/semantic formats — do not invent a parallel abstraction when gopass already defines the concept. When adding a feature gopass has, check gopass's source and match its schema/semantics.
- SPDX license headers on all source files
- Nix flake provides the full dev environment (`direnv allow` to activate)
- `gen/android/` looks like a generated directory but contains git-tracked, manually maintained files — **except `app/build.gradle.kts`, which `tauri android build` re-renders from its template every run, silently dropping manual edits.** Put manual gradle config (e.g. `applicationIdSuffix`) in `gen/android/settings.gradle` instead, which Tauri does not regenerate.
- Tauri v2 IPC naming: Rust uses `snake_case`, frontend/Kotlin use `camelCase` — Tauri auto-converts at the boundary. Match the existing plugin code.
- The Android debug build sets `applicationIdSuffix = ".debug"` (installs as `xyz.yzx9.gpm.debug`) so it coexists with the release — install a debug build for diagnostics without uninstalling.
- Update `CHANGELOG.md` when adding user-facing changes. Keep entries user-focused (no technical internals).

## Docs — specs, RFCs, ADRs

Product and design knowledge lives under `docs/` in three layers, each with its own number prefix so a bare number is never ambiguous across layers:

- **`docs/specs/` — feature PRDs (product requirements).** One subdirectory per feature (`NNN-<slug>/prd.md`, bare 3-digit `001`–`NNN`) holding functional + non-functional requirements and user characteristics — _not_ implementation. How something is built lives in the code/git; keep "current state" to a few sentences. Start from `docs/specs/000-template/` — only `prd.md` is required; `design.md` / `security.md` / `research.md` are optional companions.

  **Personas** — Jordan (primary, self-hosting gopass user) and Casey (secondary, mobile-first newcomer), plus anti-personas — live in [`docs/personas.md`](docs/personas.md). Each PRD's Use-Cases notes how they act _in that feature_, not who they are.

- **`docs/rfcs/` — design RFCs.** The "how + why" for a piece of work: design rationale, alternatives considered, effort, and Priority / Status / Phase. One file per RFC (`RNNN-<slug>.md`, `R`-prefixed 3-digit). The name is a slight misnomer — these aren't IETF Requests for Comments; "RFC" is kept for familiarity (see `docs/rfcs/R000-template.md` for the rationale). When the RFC's feature ships, delete the file — the rationale then lives in the code / threat model / the feature's `design.md`, and the numbering gaps this leaves are expected.

  **Before writing an RFC.** Read the feature spec it serves (`docs/specs/NNN-*`) first — the PRD's personas, use-cases, and product-level "what/why" are the macro context the design answers to. An RFC is the _how_ for an already-defined _what_; drafting one without reading the spec re-derives the goals in a vacuum and tends to optimize around the narrow technical problem rather than the actual objective. If the feature has no spec, write or scope the spec first.

- **`docs/adr/` — architecture decision records.** Foundational, cross-cutting, hard-to-reverse choices (`ANNN-<slug>.md`, `A`-prefixed 3-digit). Frozen and append-only — mark `Superseded` if reversed, never delete or rewrite.

  **When to write an ADR.** Write one when a decision is all three: _foundational / cross-cutting_ (it shapes many features, not one), _hard to reverse_ (undoing it means re-architecting), and _not recoverable from the code_. Examples: the Tauri/Rust/age-only stack (A001), rust-first-without-gopass (A002), config-storage tiering (A003). Don't write an ADR for: a single feature's design (use an RFC, or the feature's `design.md`), a small reversible choice (just do it, or open an Issue), or product behavior (that's a spec).

Principles:

- Write down what the code/git cannot reconstruct: requirements, user personas, and the _why_ of non-obvious decisions. Don't duplicate implementation detail.
- Number prefixes disambiguate across layers: bare `NNN` = spec/feature, `RNNN` = RFC, `ANNN` = ADR.
- Reference direction: any doc may cite an ADR, and RFCs may reference features — but **feature docs do not reference RFCs** (which RFC implements a feature is read from the code, not asserted in the PRD). Don't reference temporary planning artifacts in code, commit messages, or docs — write self-contained explanations of the what and why.

## Compact Instructions

When compressing, preserve in priority order:

1. Architecture decisions (NEVER summarize)
2. Modified files and their key changes
3. Current verification status (pass/fail)
4. Open TODOs and rollback notes
5. Tool outputs (can delete, keep pass/fail only)
