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
