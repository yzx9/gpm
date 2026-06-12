# Encrypted SSH private key support

**Priority:** P2
**Status:** TODO
**Phase:** Post-MVP (v1.1)

## What

Support passphrase-encrypted SSH private keys as age identities. Currently `age::ssh::Identity::Encrypted` is explicitly rejected at `crypto.rs:67`. This plan enables it.

## Why

Users may have SSH private keys that are passphrase-protected (the standard OpenSSH key format with `-----BEGIN ENCRYPTED ...-----` or `Proc-Type: 4,ENCRYPTED`). Currently these keys cannot be used with gpm — the user must export a passphrase-less copy, which is a security downgrade.

age natively supports encrypted SSH keys via `age::ssh::Identity::from_buffer(buf, Some(passphrase))`. We just need to accept the passphrase and pass it through.

## Context

### Current behavior

```rust
// crypto.rs:67-72
age::ssh::Identity::Encrypted(_) => {
    return Err(Error::new(
        ErrorCode::InvalidIdentity,
        "Encrypted SSH keys are not yet supported as age identities",
    ));
}
```

### Target behavior

```
setup() → user pastes SSH private key → detect encrypted → prompt for passphrase
decrypt() → provide passphrase → age::ssh::Identity::from_buffer(buf, Some(passphrase))
```

### Relationship to 0002

Plan 0002 (age encrypted identity) protects x25519 identities at rest using age's passphrase format. This plan (0003) handles SSH keys that have their own passphrase built into the key format. They are orthogonal:

- 0002: protects identity FILE on disk (storage layer)
- 0003: supports SSH key FORMAT that has its own passphrase (key layer)

Both can coexist: an encrypted SSH key could be further protected by 0002's storage-layer encryption.

## Implementation

1. **`rustpass/src/crypto.rs`** — Handle `age::ssh::Identity::Encrypted`:
   - `decrypt_bytes()` accepts optional passphrase parameter
   - Pass to `age::ssh::Identity::from_buffer(buf, passphrase)`
   - Remove the hard rejection of encrypted SSH keys

2. **`rustpass/src/store.rs`** — Passphrase plumbing:
   - `get()` needs access to the SSH key passphrase
   - Same pattern as 0002: passphrase cached in AppState, injected as parameter

3. **`src-tauri/src/lib.rs`** — New commands:
   - `set_ssh_passphrase(passphrase: String)` — store passphrase for encrypted SSH key
   - Or: reuse the unlock flow from 0002 if the passphrase is the same

4. **Frontend** — Setup flow update:
   - Detect encrypted SSH key during setup
   - Prompt for passphrase
   - Store passphrase (encrypted) alongside identity config

### Key design question

Should the SSH key passphrase be the same as the age identity passphrase (from 0002)? Or separate? Options:

- **Same passphrase**: Simpler UX. One unlock covers both. But if the SSH key was encrypted with a different passphrase externally, this won't work.
- **Separate passphrase**: More flexible. Supports externally-encrypted SSH keys. But two passwords to manage.
- **Auto-detect**: If SSH key is encrypted, prompt for its specific passphrase during setup. Store it alongside. Unlock flow provides all stored passphrases.

Recommended: auto-detect. The passphrase is a property of the SSH key, not the user's choice.

## Effort

~0.5-1 day (human) / ~15 min (CC)

## Depends on

None — independent of 0002.

---

## Review Decisions (eng-review 2026-06-12)

### D1: Encrypted SSH key detection

**Decision: Backend validate command.** New `validate_identity` Tauri command that parses identity and returns `{ type, encrypted }`. Frontend prompts for passphrase only when `encrypted=true`. OpenSSH encrypted/unencrypted keys share the same header — detection requires Rust-side parsing.

### D2: Passphrase caching

**Decision: Unified cached passphrase.** `Store` gets `cached_passphrase: RwLock<Option<Zeroizing<String>>>`. Used for both age identity decryption and SSH key parsing. Populated during `unlock()`, zeroized on `lock()`. For dual encryption (age + SSH, different passphrases), try unified passphrase first, surface specific error if SSH parsing fails.

### D3: Test coverage

**Decision: Full coverage — 16 tests.** Includes crypto (encrypted SSH decrypt happy/wrong/missing passphrase, RSA variant), store (lifecycle, dual encryption, lock zeroize), IPC (`validate_identity`), and frontend (encrypted SSH detection).

### D4: Pre-existing unlock bug fix

**Decision: Fix in this PR.** `unlock()` command at `lib.rs:213` creates `Store::new(store_dir)` inside `spawn_blocking` — caches identity in throwaway instance that's immediately dropped. `state.store` cache is never populated. Fix: add `Store::set_cached_identity()` method, call from unlock command after `spawn_blocking` returns. Also fix auto-lock timer (same pattern).

### Outside voice findings (codex)

1. **recipient.rs:186** also hard-rejects encrypted SSH keys — must update `identity_to_recipient()` to accept passphrase.
2. **AuthState.encrypted** only means "age-encrypted at rest." Encrypted SSH key stored as plaintext has `encrypted=false`. Must extend `AuthState` to represent "passphrase required for any reason."
3. **Plan numbering error:** file is 0002 but references itself as 0003.

## NOT in scope

- Decrypting SSH keys at setup time (re-exporting unencrypted PEM) — deferred to avoid ssh_key crate serialization complexity
- Storing SSH passphrase on disk — session-only cache, re-entered on each app launch
- Bulk entry decryption optimization — acceptable ~100ms latency per `get()` call for single-entry password manager use case
- Encrypted PKCS#8 keys (`-----BEGIN ENCRYPTED PRIVATE KEY-----`) — age library doesn't support these
- Multi-identity support (plan 0007)

## What already exists

- `crypto.rs:62-88`: SSH identity parsing path — calls `from_buffer(buf, None)`, rejects `Encrypted` variant. **Plan reuses this path** with passphrase parameter.
- `store.rs:102-124`: `unlock()/lock()/cached_identity` pattern for age-encrypted identities. **Plan extends** with `cached_passphrase`.
- `recipient.rs:165-210`: `identity_to_recipient()` — derives public key from private key. **Plan must update** to handle encrypted SSH keys.
- `SetupPage.vue:364-377`: SSH key passphrase field for Git auth. **Plan does NOT reuse** — this is for Git clone/pull, not age identity decryption.
- `UnlockPage.vue`: Passphrase prompt for age-encrypted identities. **Plan extends** for unified passphrase.

## Implementation Tasks

- [ ] **T1 (P1, human: ~30min / CC: ~5min)** — lib.rs+store.rs — Fix unlock throwaway Store bug
  - Surfaced by: Outside voice (codex) #3
  - Files: `src-tauri/src/lib.rs`, `rustpass/src/store.rs`
  - Verify: `just test` + manual: unlock encrypted identity → decrypt entry

- [ ] **T2 (P1, human: ~30min / CC: ~5min)** — recipient.rs — Update identity_to_recipient for encrypted SSH keys
  - Surfaced by: Outside voice (codex) #1
  - Files: `rustpass/src/recipient.rs`, `rustpass/src/store.rs`
  - Verify: unit test: `identity_to_recipient(encrypted_key)` returns recipient

- [ ] **T3 (P1, human: ~1h / CC: ~10min)** — crypto.rs — Add ssh_passphrase param, handle Identity::Encrypted
  - Surfaced by: Plan step 1
  - Files: `rustpass/src/crypto.rs`
  - Verify: `just test` + new encrypted SSH key decrypt tests

- [ ] **T4 (P1, human: ~1h / CC: ~10min)** — store.rs — Add cached_passphrase, thread through get()
  - Surfaced by: D2 decision
  - Files: `rustpass/src/store.rs`
  - Verify: `just test` + new store lifecycle tests

- [ ] **T5 (P1, human: ~1h / CC: ~10min)** — lib.rs+frontend — Add validate_identity command + detection UX
  - Surfaced by: D1 decision
  - Files: `src-tauri/src/lib.rs`, `src/pages/SetupPage.vue`, `src/types.ts`
  - Verify: manual: paste encrypted SSH key → passphrase prompt appears

- [ ] **T6 (P1, human: ~30min / CC: ~5min)** — lib.rs+frontend — Extend AuthState for encrypted SSH key
  - Surfaced by: Outside voice (codex) #4
  - Files: `src-tauri/src/lib.rs`, `src/types.ts`, `src/main.ts`
  - Verify: encrypted SSH key identity → router redirects to unlock page

- [ ] **T7 (P1, human: ~2h / CC: ~5min)** — tests — Add 16 tests for full coverage
  - Surfaced by: D3 decision
  - Files: `rustpass/tests/`, `src/pages/SetupPage.test.ts`
  - Verify: `just test` passes all new tests

- [ ] **T8 (P1, human: ~1h / CC: ~10min)** — lib.rs+store.rs+frontend — Update unlock flow for unified passphrase
  - Surfaced by: D2 decision + D4 fix
  - Files: `src-tauri/src/lib.rs`, `rustpass/src/store.rs`, `src/pages/UnlockPage.vue`
  - Verify: unlock with passphrase → decrypt entry → lock → decrypt fails

## Failure Modes

| Codepath                                              | Failure                                              | Test covers?      | Error handling?                    | User sees                                    |
| ----------------------------------------------------- | ---------------------------------------------------- | ----------------- | ---------------------------------- | -------------------------------------------- |
| `decrypt_bytes` encrypted SSH + wrong passphrase      | bcrypt-pbkdf fails                                   | **YES** (planned) | `WrongPassphrase` error            | "Wrong passphrase" message on unlock page    |
| `decrypt_bytes` encrypted SSH + no passphrase         | `Identity::Encrypted` not handled                    | **YES** (planned) | `InvalidIdentity` error            | "Encrypted SSH keys need a passphrase"       |
| `unlock` with dual encryption, different passphrases  | SSH parse fails after age decrypt                    | **YES** (planned) | Need new error path                | Specific error: "SSH key passphrase differs" |
| `identity_to_recipient` with encrypted SSH key        | Can't derive recipient without passphrase            | **YES** (planned) | Accept passphrase param            | Setup validates identity before saving       |
| `get()` after app restart with cached passphrase lost | Cache empty, identity is plaintext encrypted SSH key | **YES** (planned) | AuthState triggers unlock redirect | Redirected to unlock page                    |
| Auto-lock timer fires                                 | `lock()` on throwaway Store (bug)                    | **YES** (T1 fix)  | Fix: clear cache on state.store    | Timer works correctly                        |

## Worktree Parallelization Strategy

| Step                          | Modules touched            | Depends on |
| ----------------------------- | -------------------------- | ---------- |
| T1: Fix unlock bug            | store.rs, lib.rs           | —          |
| T2: Update recipient.rs       | recipient.rs, store.rs     | —          |
| T3: crypto.rs changes         | crypto.rs                  | —          |
| T4: Store passphrase cache    | store.rs                   | T3         |
| T5: validate_identity command | lib.rs, frontend           | T2         |
| T6: AuthState extension       | lib.rs, frontend           | T4         |
| T7: Tests                     | tests/                     | T1–T6      |
| T8: Unlock flow update        | lib.rs, store.rs, frontend | T1, T4     |

**Lane A (sequential):** T1 → T4 → T8 (store.rs changes, shared module)
**Lane B (parallel with A):** T2 → T5 (recipient + validate, independent of store.rs cache)
**Lane C (parallel with A):** T3 (crypto.rs, independent)
**After A+B+C merge:** T6 → T7 (depend on all prior work)

**Execution order:** Launch A, B, C in parallel. Merge all. Then T6, T7.

**Conflict flags:** Lanes A and B both touch store.rs (T4 and T2 modify different functions). Coordinate or run sequentially.

## GSTACK REVIEW REPORT

| Review        | Trigger               | Why                             | Runs      | Status       | Findings                                                          |
| ------------- | --------------------- | ------------------------------- | --------- | ------------ | ----------------------------------------------------------------- |
| Eng Review    | `/plan-eng-review`    | Architecture & tests (required) | 1         | ISSUES_OPEN  | 6 issues, 3 critical gaps                                         |
| Outside Voice | codex                 | Independent 2nd opinion         | 1         | ISSUES_FOUND | 6 findings, 3 critical (throwaway Store, recipient.rs, AuthState) |
| CEO Review    | `/plan-ceo-review`    | Scope & strategy                | 0         | —            | —                                                                 |
| Design Review | `/plan-design-review` | UI/UX gaps                      | 1 (stale) | —            | score: 4/10 → 7/10 (from Jun 5, pre-encrypted-SSH)                |
| Codex Review  | `/codex review`       | Independent 2nd opinion         | 1         | ISSUES_FOUND | See Outside Voice row                                             |

**CROSS-MODEL:** Review and codex agree on recipient.rs gap and auth state model. Disagreement on unified vs separate passphrase (user decided unified).

**UNRESOLVED:** 0

**VERDICT:** ENG REVIEW — 6 issues found, 3 critical. Fix throwaway Store bug (T1), update recipient.rs (T2), extend AuthState (T6) before implementing feature. Ready to implement after plan update.
