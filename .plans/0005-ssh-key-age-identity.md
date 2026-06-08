# 0006: Support SSH keys as age identity

**Priority:** P2
**Status:** TODO
**Phase:** Post-MVP (v0.2)

## What

Allow users to decrypt `.age` files using SSH private keys (ed25519, RSA) in addition to native age identities (`AGE-SECRET-KEY-...`). Many gopass users encrypt with SSH public keys and decrypt with corresponding SSH private keys.

## Why

The `age` crate (str4d/rage) supports SSH keys natively via its `ssh` feature flag, but gpm currently hard-rejects any identity that doesn't start with `AGE-SECRET-KEY-`. This excludes a large portion of gopass users who use SSH key pairs for age encryption.

## Context

The `age` crate's `IdentityFile::from_buffer()` is polymorphic — it already handles both native age keys and OpenSSH private keys when the `ssh` feature is enabled. The changes are minimal:

### Implementation

1. **Enable `ssh` feature in `Cargo.toml`:**

   ```toml
   # rustpass/Cargo.toml
   age = { version = "0.11", features = ["armor", "ssh"] }
   ```

2. **Remove hard-coded validation in `store.rs`:**

   ```rust
   // REMOVE this check:
   if !identity.trim().starts_with("AGE-SECRET-KEY-") { ... }

   // REPLACE with: try parsing as identity file (supports both formats)
   ```

3. **Update error messages in `crypto.rs`** — mention "AGE-SECRET-KEY or OpenSSH private key" instead of "AGE-SECRET-KEY only"

4. **Update `SetupPage.vue`** — change identity input placeholder/hint to mention SSH key support

### Supported formats

- `AGE-SECRET-KEY-1...` — native age x25519 identity (current)
- `-----BEGIN OPENSSH PRIVATE KEY-----` — ed25519 or RSA SSH private key (new)

### Key files

- `rustpass/Cargo.toml` — Add `ssh` feature to age dependency
- `rustpass/src/store.rs` — Remove `starts_with("AGE-SECRET-KEY-")` check in `configure()`
- `rustpass/src/crypto.rs` — Update error messages in `decrypt_bytes()`
- `src/SetupPage.vue` — Update identity input placeholder text

## Effort

~0.5-1 day (human) / ~15 min (CC) — mostly removing restrictions, not adding code

## Depends on

None — independent of other plans.
