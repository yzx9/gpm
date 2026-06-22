# ADR 0001: gpm Foundational Architecture — Tauri v2 + Rust + Vue 3, Age-Only, Read-Only MVP

**Status:** Accepted

**Date:** 2026-06-05

**Context**

There is no Android GUI client that can read age-encrypted gopass/password-store repositories. Android Password Store is unmaintained and GPG-only. gopass is Go/CLI-only. The intersection of age encryption + gopass store format + Android is empty.

## Decision

Build gpm as an Android-first, age-only, read-only gopass password client using **Tauri v2 + Rust + Vue 3**, with a "core-first, mobile later" build sequence.

This ADR records the foundational architectural choices.

## Foundational Premises

The decision rests on five assumptions, recorded so any of them can be revisited if it turns out wrong:

1. **Tauri v2 mobile is mature enough for a read-only password viewer** — no Autofill, no complex native UI, no editing, which avoids the areas where Tauri mobile is roughest.
2. **The Rust `age` crate (str4d/rage) is the right crypto choice** — one reference implementation shared across desktop and Android, no Kotlin crypto to audit.
3. **The gopass age-store format is stable enough to build against** (`.age` files in git; first line is the password). The single configured identity is tried against every `.age` file — multi-identity and recipient-aware decryption remain deferred.
4. **Local config + zeroize-per-decrypt for MVP identity storage, with Android Keystore deferred** — zeroize is ~20 lines of pure Rust with no Android dependency, whereas Keystore adds ~1 day of platform debugging on top of an already ambitious build. Keystore later arrived via the biometric-keystore and secure-keystore plugins.
5. **Read-only is the right MVP scope** — ship clone/list/search/decrypt/copy, get feedback, then expand. (Secret creation/write has since shipped; edit/delete remain in progress.)

## Technology Stack

**Framework: Tauri v2 (Rust backend + WebView frontend)**

- Single Rust codebase for desktop and Android. No Kotlin crypto, no platform-specific logic for core operations.
- WebView UI via Vue 3 + TypeScript + Vite. Mobile-first responsive layout.
- Tauri v2 mobile target for Android. Desktop as bonus, not primary target.

**Crypto: Rust `age` crate (from str4d/rage)**

- Reference-quality Rust implementation. One crypto library across all platforms.
- No `secrecy` crate (blocks `Serialize`, fights Tauri IPC). Custom `Debug` impl with `[REDACTED]` instead.

**Git: `git2` crate (vendored OpenSSL + libgit2)**

- Clone and fast-forward-only pull. HTTPS with PAT authentication.
- Vendored builds for Android cross-compilation compatibility.

**Clipboard: `tauri-plugin-clipboard-manager`**

- Existing Tauri v2 plugin with Android support. No custom Kotlin clipboard code for MVP.
- 30-second auto-clear via `tokio::time::sleep` + clipboard overwrite.

## Scope Constraints

| Constraint      | Decision                                          | Rationale                                                |
| --------------- | ------------------------------------------------- | -------------------------------------------------------- |
| Read-only MVP   | Clone, pull, list, search, decrypt, copy only     | Ship narrow, get feedback, then expand                   |
| Age-only        | No `.gpg`, no GPG, no gopass mounts               | Age is simpler, growing user base, avoids GPG complexity |
| No cloud sync   | Git is the only sync mechanism                    | No third-party servers, no analytics                     |
| Single identity | One age identity, tried against every `.age` file | No `.age-recipients` parsing needed for MVP              |
| Single repo     | One repo + one identity, re-setup to change       | Settings page and multi-repo are post-MVP                |
| HTTPS-only git  | PAT for authentication                            | SSH requires libssh2 NDK cross-compilation (post-MVP)    |

## Security Architecture

**Core principle: trust IS the product.** The app is auditable — minimal attack surface, no data leaves the device.

### Operation Split by Sensitivity

Two operations handle decrypted passwords, with different trust boundary profiles:

| Operation                   | Password crosses IPC?                                 | Trust boundaries touched                 | Usage frequency |
| --------------------------- | ----------------------------------------------------- | ---------------------------------------- | --------------- |
| `copy_password` (primary)   | **No** — Rust decrypt → clipboard → zeroize           | Rust memory → Kotlin → Android clipboard | ~90% of usage   |
| `show_password` (secondary) | **Yes** — `SensitiveContent` with `Zeroizing<String>` | Rust memory → IPC → WebView JS heap      | ~10% of usage   |

The primary operation keeps the password entirely within Rust memory (zeroizable) → Kotlin (ByteArray zeroizable) → Android clipboard (30s overwrite). The JS heap is never involved.

### Memory Safety Pattern

- `Zeroizing<String>` on all decrypted content and identity bytes — zeroized on Drop
- Identity loaded from secure storage internally, never passed from Vue
- Identity zeroized after every decrypt call
- `DecryptedEntry` custom `Debug` impl returns `[REDACTED]`
- Error messages sanitized — no secrets, no identity content, no raw decrypted text

### Trust Boundaries

Seven boundaries documented with zeroize capability and risk level:

1. **Rust internal memory** — ✅ full zeroize control
2. **Rust → WebView IPC** (show_password only) — ❌ JS strings immutable, GC-managed
3. **Rust → Kotlin (JNI)** — ✅ ByteArray zeroizable, never String
4. **Tauri plugin bridge buffer** — ⚠️ controlled by Tauri internals
5. **JVM heap** (clipboard plugin) — ❌ JVM String immutable (accept for MVP)
6. **WebView JS heap** (show only) — ❌ GC, no overwrite (acknowledged limitation)
7. **Android clipboard** — ✅ 30s overwrite by design

### Vue Security Protocol (show_password)

- 30-second auto-clear timer
- `onBeforeUnmount` + `onBeforeRouteLeave` null all reactive refs
- No `localStorage`, no `<KeepAlive>`, no password props, no `console.log` with secrets

## Build Sequence

**Approach A: Core-first, mobile later** (chosen over vertical-slice-on-Android and plugin-first)

1. **Phase 1: Rust core library** — crypto + store parsing + git, tested on desktop with real `.age` fixtures
2. **Phase 2: Desktop Tauri app** — wire Rust commands, build 3-page Vue UI
3. **Phase 3: Android target** — `tauri android init`, mobile plugin interface, test on device
4. **Phase 4: Polish & publish** — FLAG_SECURE, APK signing, GitHub Actions release

Rationale: Rust core is platform-agnostic. Prove crypto works on desktop in hours, then Android is an engineering task (not a research task).

## Rust Command API

```rust
// Non-sensitive
fn clone_repo(url: String, dest: String) -> Result<(), AppError>
fn pull_repo(repo_path: String) -> Result<PullResult, AppError>
fn list_entries(repo_path: String) -> Result<Vec<Entry>, AppError>

// Primary sensitive — password never crosses IPC
fn copy_password(repo_path: String, entry_path: String) -> Result<CopyResult, AppError>

// Secondary sensitive — password crosses IPC with strict lifecycle
fn show_password(repo_path: String, entry_path: String) -> Result<SensitiveContent, AppError>
```

## Consequences

- **Compact backend:** 1,537 lines of Rust for full read-only password manager functionality.
- **Strong security posture:** Full-chain zeroize on the primary operation. Acknowledged limitations on the secondary operation (JS heap, JVM heap).
- **Android-first but not Android-only:** Same Rust core runs on desktop. Build once, target both.
- **Age-only limitation:** GPG users cannot use gpm. See ADR 0002 for the decision not to integrate gopass (which would add GPG support).
- **Read-only limitation:** Write operations (create, edit, delete) are post-MVP. Will be implemented in Rust using existing `age` + `git2` crates.
