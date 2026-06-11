# Passphrase-encrypted age identity file

**Priority:** P2
**Status:** TODO
**Phase:** Post-MVP (v1.1)

## What

Use age's native passphrase-encrypted identity file format to protect the x25519 identity at rest. This is the same format produced by `age-keygen | age -p > key.age` and supported by the age crate's `age::encrypted::Identity`.

## Why

Currently the age identity (`AGE-SECRET-KEY-1...`) is stored as plaintext in `{config_dir}/identity`. On Android, this is insufficient:

- Rooted device: identity file directly readable
- Backup extraction: identity included in app backups
- Stolen device with exploit: identity accessible without user interaction

age natively supports passphrase-encrypted identity files — a standard age encrypted file (scrypt recipient) containing the identity. This is well-audited, interoperable with the age CLI, and requires no new crypto dependencies.

## Context

### Current flow

```
setup() → save_identity(plaintext) → fs::write(identity_path, raw_bytes)
decrypt() → fs::read(identity_path) → age::decrypt(encrypted_file, identity)
```

### Target flow

```
setup() → user sets passphrase → encrypt identity with age scrypt → fs::write(identity_path, encrypted_blob)
unlock(passphrase) → decrypt identity from file → cache in Store (Zeroizing<Vec<u8>>)
decrypt() → use cached identity → age::decrypt(encrypted_file, identity)
```

### age native encrypted identity

The age crate provides `age::encrypted::Identity` which implements the `age::Identity` trait. It reads a passphrase-encrypted age file and decrypts it to obtain identity entries. The file format is standard:

```
-----BEGIN AGE ENCRYPTED FILE-----
YWdlLWVuY3J5cHRpb24ub3JnL3YxCi0+IHNjcnlwdCA...
-----END AGE ENCRYPTED FILE-----
```

This is interoperable with `age -d -i key.age` on the command line.

### Key design decisions (from engineering review)

1. **Direct scrypt**: Use `age::scrypt::Recipient` / `age::scrypt::Identity` directly (not `age::encrypted::Identity` which requires Callbacks trait). Accepts passphrase as `&str`, no interactive prompting needed.

2. **rustpass owns the cache**: Store gains `unlock(passphrase)` / `lock()` methods with internal `cached_identity` field. Store::get() checks cache first, falls back to loading from disk. No API signature change. Tauri layer stays thin IPC wrapper. Supersedes earlier IdentitySource enum idea.

3. **`Zeroizing<Vec<u8>>`**: Cache type matches `config::load_identity()` return type and `crypto::decrypt_bytes()` parameter type. Not `Zeroizing<String>`.

4. **`RwLock` for cache**: `RwLock<Option<Zeroizing<Vec<u8>>>>` in Store for concurrent reads of cached identity. Timer handle stored alongside.

5. **Cancel-and-respawn timer**: Sliding 5-minute timer stores `JoinHandle`. On each reset, abort old timer and spawn new one. Only the latest timer fires. Not fire-and-forget (which would cause premature locks).

6. **Optional passphrase**: Passphrase is optional during setup. If skipped, identity stored as plaintext (same as today). Warning shown when skipping.

7. **Empty passphrase rejected**: Both frontend and backend validate that passphrase is non-empty when encryption is requested. `age::scrypt` accepts empty string — we must not let it produce a trivially-decryptable file.

8. **`classify_identity()` helper**: New shared function returns `IdentityType` enum (X25519 | SshEd25519 | SshRsa | AgeEncrypted | Unknown). Eliminates prefix-check duplication across 5+ call sites. Corresponding TypeScript helper.

9. **`get_auth_state()` command**: Single Tauri command returns `{ configured: bool, encrypted: bool, unlocked: bool }`. One IPC call instead of three. Atomic snapshot, no TOCTOU race.

10. **Atomic write**: `save_identity()` and `change_passphrase()` use write-to-temp + rename pattern. Prevents identity file corruption on write failure.

11. **Async unlock**: `unlock()` command is async with `tokio::spawn_blocking` for scrypt. UnlockPage shows loading spinner. Scrypt targets ~1s on encrypting device, may be 2-5s on low-end Android.

12. **Full test coverage**: All ~45 new codepaths get tests. Unit tests for crypto/config, integration tests for Store unlock/lock flow, Vue component tests for unlock page.

13. **Lock event**: When timer fires and locks, emit a Tauri event so mounted pages can redirect. Route guard alone is insufficient — it only runs on navigation.

14. **Reset clears cache**: `Store::reset()` also calls `lock()` (zeroize cache + cancel timer). Otherwise stale cache survives reset.

### SSH keys

This plan covers x25519 identities only. SSH private keys are handled by plan 0003 (encrypted SSH key support via `age::ssh::Identity::Encrypted`). For storage-layer protection, both identity types are encrypted with age scrypt before writing to disk — the encryption wraps the entire identity file regardless of content type.

## Implementation

### Rust core (`rustpass/`)

1. **`identity.rs`** (new) — Identity type classification:
   - `IdentityType` enum: `X25519`, `SshEd25519`, `SshRsa`, `AgeEncrypted`, `Unknown`
   - `classify_identity(bytes: &[u8]) -> IdentityType` — prefix-based detection
   - Shared across crypto.rs, config.rs, store.rs

2. **`crypto.rs`** — Add identity encryption/decryption using direct scrypt:
   - `encrypt_identity(passphrase: &str, identity: &[u8]) -> Result<Vec<u8>>` — `age::scrypt::Recipient` + armor
   - `decrypt_identity(passphrase: &str, encrypted: &[u8]) -> Result<Vec<u8>>` — `age::scrypt::Identity` + dearmor
   - Both reject empty passphrase with `IdentityNotEncrypted` error

3. **`config.rs`** — Encrypted identity storage:
   - `save_identity()` accepts optional passphrase, encrypts if present
   - `save_identity_raw()` → atomic write (temp + rename pattern)
   - `is_identity_encrypted()` → check file prefix via `classify_identity()`
   - `load_identity()` unchanged — returns raw bytes (encrypted or plaintext)

4. **`store.rs`** — Identity cache + unlock/lock:
   - New fields: `cached_identity: RwLock<Option<Zeroizing<Vec<u8>>>>`, `lock_timer: Mutex<Option<JoinHandle<()>>>`
   - `unlock(passphrase: &str)` — decrypt identity, store in cache, start sliding timer
   - `lock()` — zeroize cache, cancel timer
   - `is_identity_encrypted()` → delegates to config
   - `is_unlocked()` → checks `cached_identity`
   - `get()` modified: check `cached_identity` first, fall back to `config.load_identity()` if None
   - `reset()` also calls `lock()` to clear cache and cancel timer
   - Timer: cancel-and-respawn on each `copy_password`/`show_password` call, emit Tauri event on expiry

5. **`error.rs`** — New error codes:
   - `IdentityEncrypted` — identity requires passphrase to decrypt
   - `WrongPassphrase` — passphrase does not match encrypted identity
   - `IdentityNotEncrypted` — operation requires encrypted identity, or empty passphrase rejected

### Tauri layer (`src-tauri/`)

6. **`lib.rs`** — New commands + state:
   - `get_auth_state() -> AuthState` — single command, returns `{ configured, encrypted, unlocked }`
   - `unlock(passphrase: String)` — async, calls `store.unlock()`, loading spinner in UI
   - `lock()` — calls `store.lock()`
   - `set_passphrase(passphrase: String)` — encrypt existing plaintext identity, rejects empty
   - `change_passphrase(old: String, new: String)` — re-encrypt identity, atomic write
   - `is_identity_encrypted() -> bool` — for cases that need just this one boolean
   - Modify `copy_password`/`show_password` to reset sliding timer after use
   - AppState unchanged (no cache — cache lives in Store)

### Frontend (`src/`)

7. **`types.ts`** — New IPC types:
   - `AuthState { configured: boolean, encrypted: boolean, unlocked: boolean }`

8. **`main.ts`** — Router guard update:
   - Call `get_auth_state()` instead of `is_configured`
   - Redirect flow: not configured → setup, encrypted + locked → unlock, else → allow

9. **`pages/SetupPage.vue`** — Optional passphrase during setup:
   - After identity paste, optional passphrase input
   - Empty = plaintext (with warning), filled = encrypted
   - Frontend validates non-empty if filled

10. **`pages/UnlockPage.vue`** (new) — Passphrase entry on app launch:
    - Shown when identity is encrypted but not yet unlocked
    - Input → `unlock()` with loading spinner → navigate to entries
    - Error display for wrong passphrase

11. **`pages/SettingsPage.vue`** — Passphrase management:
    - Set passphrase (plaintext → encrypted)
    - Change passphrase (old + new)
    - Clear passphrase (encrypted → plaintext with confirmation)

12. **`main.ts`** — Lock event listener:
    - Listen for Tauri event from timer expiry
    - Redirect to unlock page when lock fires on mounted pages

## Effort

~2-3 days (human) / ~1 hour (CC)

## Depends on

None — this is the foundation for 0004 (biometric unlock).

## NOT in scope

- **Encrypting repo.json credentials** (PAT, SSH key, SSH passphrase stored plaintext in repo.json) — valid security concern but separate from identity encryption. Deferred.
- **Lock-on-background** (Android app lifecycle integration) — would require new Android plugin code. Sliding timer is sufficient for now.
- **Biometric unlock** — plan 0004, builds on this plan's unlock/lock infrastructure.
- **Multiple identity support** — deferred, single identity model preserved.
- **Scrypt performance benchmarking on low-end Android** — recommended but not blocking. The async spinner handles the latency; measurements can inform future work factor tuning.

## What already exists

- **`crypto::decrypt_bytes()`** — already accepts identity bytes as parameter and handles x25519/SSH routing. The new `encrypt_identity()`/`decrypt_identity()` functions are additive, no changes to existing decryption path.
- **`config::save_identity()` / `load_identity()`** — existing identity persistence. Modified to support optional encryption but backward-compatible (plaintext path unchanged).
- **`Store::get()`** — loads identity internally. Modified to check cache first but falls back to existing disk-load path. No signature change.
- **Router guard** — existing `is_configured` check. Replaced by `get_auth_state()` which is a superset.
- **Test infrastructure** — `common/mod.rs` provides `generate_test_keypair()` and `encrypt_to_recipient()`. New identity encryption tests reuse these helpers.

## Implementation Tasks

Synthesized from this review's findings. Each task derives from a specific
finding above. Run with Claude Code or Codex; checkbox as you ship.

- [ ] **T1 (P1, human: ~2h / CC: ~15min)** — `identity.rs` — Add `IdentityType` enum and `classify_identity()` helper
  - Surfaced by: Code Quality — DRY violation across 5+ call sites
  - Files: `rustpass/src/identity.rs`, `rustpass/src/lib.rs`
  - Verify: `cargo test`

- [ ] **T2 (P1, human: ~3h / CC: ~20min)** — `crypto.rs` — Add `encrypt_identity()` / `decrypt_identity()` using direct scrypt
  - Surfaced by: Architecture — D2 (direct scrypt, no Callbacks adapter)
  - Files: `rustpass/src/crypto.rs`
  - Verify: Unit tests for encrypt/decrypt round-trip, wrong passphrase, empty passphrase rejection, corrupted data

- [ ] **T3 (P1, human: ~2h / CC: ~10min)** — `config.rs` — Encrypted identity storage with atomic writes
  - Surfaced by: Code Quality — D8 (empty passphrase) + D10 (atomic write)
  - Files: `rustpass/src/config.rs`
  - Verify: Unit tests for encrypt + save, load encrypted, is_identity_encrypted, atomic write failure recovery

- [ ] **T4 (P1, human: ~4h / CC: ~30min)** — `store.rs` — Identity cache, unlock/lock, cancel-and-respawn timer
  - Surfaced by: Architecture — D14 (rustpass owns cache) + Outside voice (timer cancellation)
  - Files: `rustpass/src/store.rs`
  - Verify: Integration tests for unlock/lock flow, timer reset, concurrent access, reset clears cache

- [ ] **T5 (P1, human: ~1h / CC: ~5min)** — `error.rs` — New error codes (IdentityEncrypted, WrongPassphrase, IdentityNotEncrypted)
  - Surfaced by: Plan implementation item 5
  - Files: `rustpass/src/error.rs`
  - Verify: `cargo test`

- [ ] **T6 (P1, human: ~2h / CC: ~15min)** — `lib.rs` — New Tauri commands: get_auth_state, unlock (async), lock, set_passphrase, change_passphrase
  - Surfaced by: Architecture — D9 (get_auth_state), D12 (async unlock)
  - Files: `src-tauri/src/lib.rs`
  - Verify: `cargo test`, manual Android testing of unlock latency

- [ ] **T7 (P1, human: ~1h / CC: ~10min)** — `main.ts` — Router guard update + lock event listener
  - Surfaced by: Architecture — D9 (single IPC call) + Outside voice (lock event for mounted pages)
  - Files: `src/main.ts`
  - Verify: Test navigation flows (not configured, encrypted+locked, encrypted+unlocked, plaintext)

- [ ] **T8 (P1, human: ~2h / CC: ~15min)** — `UnlockPage.vue` — New page with passphrase input + loading spinner
  - Surfaced by: Plan implementation item 10
  - Files: `src/pages/UnlockPage.vue`, `src/main.ts` (route registration)
  - Verify: Manual test, component test

- [ ] **T9 (P1, human: ~1h / CC: ~10min)** — `SetupPage.vue` — Optional passphrase field with warning
  - Surfaced by: Code Quality — D8 (frontend passphrase validation)
  - Files: `src/pages/SetupPage.vue`
  - Verify: Manual test, component test

- [ ] **T10 (P2, human: ~1h / CC: ~10min)** — `SettingsPage.vue` — Passphrase management (set/change/clear)
  - Surfaced by: Plan implementation item 11
  - Files: `src/pages/SettingsPage.vue`
  - Verify: Manual test

- [ ] **T11 (P1, human: ~1h / CC: ~5min)** — `types.ts` — Add AuthState type + TypeScript classify_identity helper
  - Surfaced by: Architecture — D9 (get_auth_state return type) + Code Quality — D7 (DRY in frontend)
  - Files: `src/types.ts`, `src/utils/identity.ts` (new)
  - Verify: TypeScript compilation

- [ ] **T12 (P1, human: ~4h / CC: ~30min)** — Full test coverage for all new codepaths
  - Surfaced by: Test Review — D11 (full coverage, ~45 new codepaths)
  - Files: `rustpass/tests/identity_encryption.rs` (new), `src/pages/UnlockPage.test.ts` (new), `src/pages/SetupPage.test.ts` (update)
  - Verify: `just test`

## Failure modes

| Codepath              | Failure mode              | Test covers? | Error handling?                         | User sees          |
| --------------------- | ------------------------- | ------------ | --------------------------------------- | ------------------ |
| `encrypt_identity()`  | Empty passphrase          | Yes          | Yes — rejected                          | Clear error        |
| `encrypt_identity()`  | Disk full during write    | Yes (atomic) | Yes — temp write fails, old file intact | Error message      |
| `decrypt_identity()`  | Wrong passphrase          | Yes          | Yes — scrypt fails                      | "Wrong passphrase" |
| `decrypt_identity()`  | Corrupted encrypted data  | Yes          | Yes — parse fails                       | Clear error        |
| `unlock()`            | Identity not encrypted    | Yes          | Yes                                     | Clear error        |
| `unlock()`            | Already unlocked          | Yes          | Idempotent                              | No-op              |
| `lock()`              | Already locked            | Yes          | Idempotent                              | No-op              |
| Timer expiry          | Fires while page mounted  | Yes          | Emits Tauri event                       | Redirect to unlock |
| `change_passphrase()` | Write fails mid-operation | Yes (atomic) | Old file intact                         | Error message      |
| `reset()`             | Forgets to clear cache    | Yes          | Calls lock()                            | Clean state        |
| `get_auth_state()`    | Cache/disk inconsistency  | No           | Returns snapshot                        | Correct state      |

## Worktree parallelization strategy

| Step                             | Modules touched              | Depends on |
| -------------------------------- | ---------------------------- | ---------- |
| T1: identity.rs                  | `rustpass/src/`              | —          |
| T2: crypto.rs encrypt/decrypt    | `rustpass/src/crypto.rs`     | T1         |
| T3: config.rs encrypted storage  | `rustpass/src/config.rs`     | T1         |
| T5: error.rs new codes           | `rustpass/src/error.rs`      | —          |
| T4: store.rs cache + unlock/lock | `rustpass/src/store.rs`      | T2, T3, T5 |
| T6: lib.rs Tauri commands        | `src-tauri/src/`             | T4         |
| T11: types.ts + identity utils   | `src/types.ts`, `src/utils/` | —          |
| T7: main.ts router guard         | `src/main.ts`                | T11        |
| T8: UnlockPage.vue               | `src/pages/`                 | T11        |
| T9: SetupPage.vue passphrase     | `src/pages/`                 | T11        |
| T10: SettingsPage.vue passphrase | `src/pages/`                 | T6         |
| T12: Tests                       | `rustpass/tests/`, `src/`    | All above  |

**Parallel lanes:**

- **Lane A:** T1 → T2 + T3 (parallel after T1) → T4 → T6
- **Lane B:** T5 (independent) → merges at T4
- **Lane C:** T11 (independent) → T7 + T8 + T9 (parallel after T11) → T10
- **Lane D:** T12 (after all lanes complete)

**Execution order:** Launch A + B + C in parallel worktrees. Merge all. Then T12.
**Conflict flags:** Lanes A and B both touch `rustpass/src/` — sequential within lane A, but B (error.rs) is a different file, so no conflict.

## GSTACK REVIEW REPORT

| Review        | Trigger               | Why                             | Runs | Status       | Findings                                                                                |
| ------------- | --------------------- | ------------------------------- | ---- | ------------ | --------------------------------------------------------------------------------------- |
| CEO Review    | `/plan-ceo-review`    | Scope & strategy                | 0    | —            | —                                                                                       |
| Codex Review  | `/codex review`       | Independent 2nd opinion         | 1    | issues_found | 3 high, 5 medium findings; timer cancellation adopted, identity cache moved to rustpass |
| Eng Review    | `/plan-eng-review`    | Architecture & tests (required) | 1    | issues_open  | 14 decisions, 0 critical gaps                                                           |
| Design Review | `/plan-design-review` | UI/UX gaps                      | 0    | —            | —                                                                                       |
| DX Review     | `/plan-devex-review`  | Developer experience gaps       | 0    | —            | —                                                                                       |

**CODEX:** Timer cancellation bug caught and fixed (cancel-and-respawn pattern). Identity cache architecture corrected (rustpass-owns, not Tauri-owns). Lock event for mounted pages added. Reset-must-clear-cache added.

**UNRESOLVED:** 0

**VERDICT:** ENG CLEARED — ready to implement.
